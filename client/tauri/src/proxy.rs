// Встроенный прокси-движок Ethernet.
//
// Все окна WebView ходят в сеть через локальный SOCKS5-релей на 127.0.0.1.
// Релей на каждое соединение читает настройки и либо подключается напрямую
// (работает с TUN/VPN), либо через пользовательский SOCKS5/HTTP-прокси с авторизацией.
// Так прокси переключается без пересоздания WebView, а логин/пароль прокси
// поддерживаются на всех платформах.

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::settings;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PING_TIMEOUT: Duration = Duration::from_secs(5);
const GEO_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_HTTP_RESPONSE_HEAD: usize = 16 * 1024;

struct Relay {
  port: u16,
  generation: watch::Sender<u64>,
}

static RELAY: OnceLock<Relay> = OnceLock::new();

/// Запускает локальный релей и возвращает его URL для `proxy_url` WebView
pub fn start() -> std::io::Result<url::Url> {
  if let Some(relay) = RELAY.get() {
    return Ok(relay_url(relay.port));
  }
  let std_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
  std_listener.set_nonblocking(true)?;
  let port = std_listener.local_addr()?.port();
  let (tx, _) = watch::channel(0u64);
  let _ = RELAY.set(Relay { port, generation: tx });

  tauri::async_runtime::spawn(async move {
    let listener = match TcpListener::from_std(std_listener) {
      Ok(l) => l,
      Err(err) => {
        log::error!("[ProxyEngine] relay listen failed: {err}");
        return;
      }
    };
    log::info!("[ProxyEngine] relay listening on 127.0.0.1:{port}");
    loop {
      let Ok((stream, _)) = listener.accept().await else { continue };
      let generation = RELAY.get().unwrap().generation.subscribe();
      tauri::async_runtime::spawn(async move {
        if let Err(err) = handle_client(stream, generation).await {
          log::debug!("[ProxyEngine] relay connection: {err}");
        }
      });
    }
  });

  Ok(relay_url(port))
}

fn relay_url(port: u16) -> url::Url {
  url::Url::parse(&format!("socks5://127.0.0.1:{port}")).unwrap()
}

/// Применяет текущие настройки сети: рвёт открытые соединения и оповещает окна
pub fn apply_network_state(app: &AppHandle) -> Value {
  if let Some(relay) = RELAY.get() {
    relay.generation.send_modify(|g| *g += 1);
  }
  let result = match settings::active_proxy() {
    Some(proxy) => {
      log::info!("[ProxyEngine] Custom proxy mode: {}", proxy.get("name").and_then(Value::as_str).unwrap_or(""));
      json!({ "mode": "custom", "proxy": proxy })
    }
    None => {
      log::info!("[ProxyEngine] Direct connection active");
      json!({ "mode": "direct" })
    }
  };
  let _ = app.emit("hermes:network-changed", ());
  result
}

// --- Локальный SOCKS5-сервер ---

async fn handle_client(mut client: TcpStream, mut generation: watch::Receiver<u64>) -> Result<(), String> {
  client.set_nodelay(true).ok();

  // Приветствие: VER, NMETHODS, METHODS
  let mut head = [0u8; 2];
  client.read_exact(&mut head).await.map_err(|e| e.to_string())?;
  if head[0] != 0x05 {
    return Err("not socks5".into());
  }
  let mut methods = vec![0u8; head[1] as usize];
  client.read_exact(&mut methods).await.map_err(|e| e.to_string())?;
  client.write_all(&[0x05, 0x00]).await.map_err(|e| e.to_string())?;

  // Запрос: VER, CMD, RSV, ATYP, DST.ADDR, DST.PORT
  let mut req = [0u8; 4];
  client.read_exact(&mut req).await.map_err(|e| e.to_string())?;
  if req[1] != 0x01 {
    client.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.ok();
    return Err("unsupported command".into());
  }
  let host = match req[3] {
    0x01 => {
      let mut ip = [0u8; 4];
      client.read_exact(&mut ip).await.map_err(|e| e.to_string())?;
      IpAddr::from(ip).to_string()
    }
    0x03 => {
      let mut len = [0u8; 1];
      client.read_exact(&mut len).await.map_err(|e| e.to_string())?;
      let mut name = vec![0u8; len[0] as usize];
      client.read_exact(&mut name).await.map_err(|e| e.to_string())?;
      String::from_utf8_lossy(&name).into_owned()
    }
    0x04 => {
      let mut ip = [0u8; 16];
      client.read_exact(&mut ip).await.map_err(|e| e.to_string())?;
      IpAddr::from(ip).to_string()
    }
    _ => return Err("bad address type".into()),
  };
  let mut port = [0u8; 2];
  client.read_exact(&mut port).await.map_err(|e| e.to_string())?;
  let port = u16::from_be_bytes(port);

  let upstream = match tokio::time::timeout(CONNECT_TIMEOUT, connect_upstream(&host, port)).await {
    Ok(result) => result,
    Err(_) => Err("timeout".to_string()),
  };
  let mut upstream = match upstream {
    Ok(stream) => stream,
    Err(reason) => {
      client.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.ok();
      return Err(format!("upstream {host}:{port}: {reason}"));
    }
  };
  upstream.set_nodelay(true).ok();
  client.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await.map_err(|e| e.to_string())?;

  // Соединение живёт до закрытия одной из сторон или до смены настроек сети
  tokio::select! {
    res = tokio::io::copy_bidirectional(&mut client, &mut upstream) => res.map(|_| ()).map_err(|e| e.to_string()),
    _ = generation.changed() => Ok(()),
  }
}

async fn connect_upstream(host: &str, port: u16) -> Result<TcpStream, String> {
  let Some(proxy) = settings::active_proxy() else {
    return TcpStream::connect((host, port)).await.map_err(|e| e.to_string());
  };
  let server = proxy.get("server").and_then(Value::as_str).unwrap_or_default();
  let proxy_port = proxy.get("port").and_then(Value::as_u64).unwrap_or(0) as u16;
  let auth = proxy.get("auth");
  let username = auth.and_then(|a| a.get("username")).and_then(Value::as_str).unwrap_or_default();
  let password = auth.and_then(|a| a.get("password")).and_then(Value::as_str).unwrap_or_default();

  let mut stream = TcpStream::connect((server, proxy_port)).await.map_err(|e| e.to_string())?;
  if proxy.get("protocol").and_then(Value::as_str) == Some("http") {
    http_connect(&mut stream, host, port, username, password).await?;
  } else {
    socks5_connect(&mut stream, host, port, username, password).await?;
  }
  Ok(stream)
}

async fn socks5_connect(s: &mut TcpStream, host: &str, port: u16, user: &str, pass: &str) -> Result<(), String> {
  let has_auth = !user.is_empty() || !pass.is_empty();
  let greeting: &[u8] = if has_auth { &[0x05, 0x02, 0x00, 0x02] } else { &[0x05, 0x01, 0x00] };
  s.write_all(greeting).await.map_err(|e| e.to_string())?;
  let mut choice = [0u8; 2];
  s.read_exact(&mut choice).await.map_err(|e| e.to_string())?;
  match choice[1] {
    0x00 => {}
    0x02 if has_auth => {
      let mut msg = vec![0x01, user.len() as u8];
      msg.extend_from_slice(user.as_bytes());
      msg.push(pass.len() as u8);
      msg.extend_from_slice(pass.as_bytes());
      s.write_all(&msg).await.map_err(|e| e.to_string())?;
      let mut status = [0u8; 2];
      s.read_exact(&mut status).await.map_err(|e| e.to_string())?;
      if status[1] != 0x00 {
        return Err("socks5 auth rejected".into());
      }
    }
    _ => return Err("socks5 no acceptable auth method".into()),
  }

  let mut req = vec![0x05, 0x01, 0x00];
  match host.parse::<IpAddr>() {
    Ok(IpAddr::V4(ip)) => {
      req.push(0x01);
      req.extend_from_slice(&ip.octets());
    }
    Ok(IpAddr::V6(ip)) => {
      req.push(0x04);
      req.extend_from_slice(&ip.octets());
    }
    Err(_) => {
      req.push(0x03);
      req.push(host.len() as u8);
      req.extend_from_slice(host.as_bytes());
    }
  }
  req.extend_from_slice(&port.to_be_bytes());
  s.write_all(&req).await.map_err(|e| e.to_string())?;

  let mut reply = [0u8; 4];
  s.read_exact(&mut reply).await.map_err(|e| e.to_string())?;
  if reply[1] != 0x00 {
    return Err(format!("socks5 connect failed: {}", reply[1]));
  }
  let skip = match reply[3] {
    0x01 => 4,
    0x04 => 16,
    0x03 => {
      let mut len = [0u8; 1];
      s.read_exact(&mut len).await.map_err(|e| e.to_string())?;
      len[0] as usize
    }
    _ => return Err("socks5 bad reply".into()),
  };
  let mut rest = vec![0u8; skip + 2];
  s.read_exact(&mut rest).await.map_err(|e| e.to_string())?;
  Ok(())
}

async fn http_connect(s: &mut TcpStream, host: &str, port: u16, user: &str, pass: &str) -> Result<(), String> {
  let target = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
  let mut req = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
  if !user.is_empty() || !pass.is_empty() {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
    req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
  }
  req.push_str("\r\n");
  s.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;

  let mut head = Vec::new();
  let mut byte = [0u8; 1];
  while !head.ends_with(b"\r\n\r\n") {
    if head.len() > MAX_HTTP_RESPONSE_HEAD {
      return Err("http proxy response too large".into());
    }
    let n = s.read(&mut byte).await.map_err(|e| e.to_string())?;
    if n == 0 {
      return Err("http proxy closed connection".into());
    }
    head.push(byte[0]);
  }
  let status_line = String::from_utf8_lossy(&head);
  let code = status_line.split_whitespace().nth(1).unwrap_or("");
  if code != "200" {
    return Err(format!("http proxy CONNECT failed: {code}"));
  }
  Ok(())
}

// --- Проверка задержки до сервера прокси по TCP ---

pub async fn test_ping(proxy: &Value) -> i64 {
  let server = proxy.get("server").and_then(Value::as_str).unwrap_or_default().to_string();
  let port = proxy.get("port").and_then(|p| p.as_u64().or_else(|| p.as_str()?.parse().ok())).unwrap_or(0);
  if server.is_empty() || port == 0 || port > 65535 {
    return -1;
  }
  let start = Instant::now();
  match tokio::time::timeout(PING_TIMEOUT, TcpStream::connect((server.as_str(), port as u16))).await {
    Ok(Ok(_)) => start.elapsed().as_millis() as i64,
    _ => -1,
  }
}

// --- Гео-IP текущего соединения (через тот же путь, что и WebView) ---

pub async fn network_geo_info() -> Value {
  let mode = if settings::active_proxy().is_some() { "custom" } else { "direct" };
  let unknown = json!({
    "ip": "Не определен",
    "country": "Неизвестно",
    "countryCode": "UN",
    "city": "",
    "org": "",
    "mode": mode,
  });

  let mut builder = reqwest::Client::builder().timeout(GEO_TIMEOUT);
  if let Some(relay) = RELAY.get() {
    if let Ok(proxy) = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", relay.port)) {
      builder = builder.proxy(proxy);
    }
  }
  let Ok(client) = builder.build() else { return unknown };

  for endpoint in ["https://ipwho.is/", "https://freeipapi.com/api/json", "https://ipapi.co/json/"] {
    let Ok(res) = client.get(endpoint).send().await else { continue };
    if !res.status().is_success() {
      continue;
    }
    let Ok(data) = res.json::<Value>().await else { continue };
    let pick = |keys: &[&str]| {
      keys.iter().find_map(|k| data.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string))
    };
    let org = data
      .get("connection")
      .and_then(|c| c.get("org"))
      .and_then(Value::as_str)
      .map(str::to_string)
      .or_else(|| pick(&["org", "isp"]))
      .unwrap_or_default();
    return json!({
      "ip": pick(&["ip", "ipAddress", "query"]).unwrap_or_else(|| "127.0.0.1".into()),
      "country": pick(&["country", "country_name", "countryName"]).unwrap_or_else(|| "Локальная сеть".into()),
      "countryCode": pick(&["country_code", "countryCode"]).unwrap_or_else(|| "UN".into()),
      "city": pick(&["city", "cityName"]).unwrap_or_default(),
      "org": org,
      "mode": mode,
    });
  }
  unknown
}

// --- Парсер ссылок прокси (SOCKS5, HTTP) ---

fn generate_id() -> String {
  uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

fn is_valid_host(host: &str) -> bool {
  !host.is_empty()
    && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '[' | ']'))
}

fn decode(s: &str) -> String {
  url::form_urlencoded::parse(format!("v={}", s.replace('+', "%2B")).as_bytes())
    .next()
    .map(|(_, v)| v.into_owned())
    .unwrap_or_default()
}

fn make_proxy(name: String, protocol: &str, server: &str, port: u16, user: String, pass: String, raw: Option<&str>) -> Value {
  let mut v = json!({
    "id": generate_id(),
    "name": name,
    "protocol": protocol,
    "server": server,
    "port": port,
  });
  if !user.is_empty() || !pass.is_empty() {
    v["auth"] = json!({ "username": user, "password": pass });
  }
  if let Some(raw) = raw {
    v["rawLink"] = json!(raw);
  }
  v
}

pub fn parse_proxy_uri(raw: &str) -> Value {
  parse_proxy_uri_inner(raw.trim()).unwrap_or(Value::Null)
}

fn parse_proxy_uri_inner(uri: &str) -> Option<Value> {
  // tg://socks?server=…&port=…&user=…&pass=…, tg://http?…, https://t.me/socks?…, https://t.me/http?…
  for (prefixes, protocol, label) in [
    (["tg://socks?", "https://t.me/socks?"], "socks5", "SOCKS5"),
    (["tg://http?", "https://t.me/http?"], "http", "HTTP"),
  ] {
    if prefixes.iter().any(|p| uri.starts_with(p)) {
      let query = &uri[uri.find('?')? + 1..];
      let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
      let get = |keys: &[&str]| {
        keys.iter().find_map(|k| params.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.clone())).unwrap_or_default()
      };
      let server = get(&["server"]);
      let port: u16 = get(&["port"]).parse().ok().filter(|p| *p > 0)?;
      if !is_valid_host(&server) {
        return None;
      }
      let name = format!("{label} {server}:{port}");
      return Some(make_proxy(name, protocol, &server, port, get(&["user", "username"]), get(&["pass", "password"]), Some(uri)));
    }
  }

  // socks5://[user:pass@]host:port[#name], socks://…, http(s)://…
  let (normalized, protocol, label, default_port) = if uri.starts_with("socks5://") || uri.starts_with("socks://") {
    (uri.replacen("socks://", "socks5://", 1), "socks5", "SOCKS5", 1080)
  } else if uri.starts_with("http://") {
    (uri.to_string(), "http", "HTTP", 8080)
  } else if uri.starts_with("https://") {
    (uri.to_string(), "http", "HTTP", 443)
  } else {
    // Простой формат host:port или user:pass@host:port
    let re = regex::Regex::new(r"^([\w.-]+:[\w.-]+@)?([a-zA-Z0-9.-]+):(\d+)$").unwrap();
    if !re.is_match(uri) {
      return None;
    }
    let parsed = url::Url::parse(&format!("socks5://{uri}")).ok()?;
    let server = parsed.host_str()?.to_string();
    let port = parsed.port().filter(|p| *p > 0)?;
    if !is_valid_host(&server) {
      return None;
    }
    let name = format!("SOCKS5 {server}:{port}");
    return Some(make_proxy(name, "socks5", &server, port, decode(parsed.username()), decode(parsed.password().unwrap_or("")), None));
  };

  let parsed = url::Url::parse(&normalized).ok()?;
  let server = parsed.host_str()?.to_string();
  let port = parsed.port().unwrap_or(default_port);
  if !is_valid_host(&server) || port == 0 {
    return None;
  }
  let name = parsed
    .fragment()
    .map(decode)
    .filter(|n| !n.is_empty())
    .unwrap_or_else(|| format!("{label} {server}:{port}"));
  Some(make_proxy(name, protocol, &server, port, decode(parsed.username()), decode(parsed.password().unwrap_or("")), Some(uri)))
}

/// Валидирует и нормализует конфиг прокси из UI
pub fn clean_proxy_config(config: &Value) -> Result<Value, String> {
  let Some(obj) = config.as_object() else { return Err("bad proxy config".into()) };
  let server = obj.get("server").and_then(Value::as_str).unwrap_or_default().trim().to_string();
  if server.is_empty() || server.contains([';', ' ', '\n']) {
    return Err("invalid proxy host".into());
  }
  let port = obj
    .get("port")
    .and_then(|p| p.as_u64().or_else(|| p.as_str().and_then(|s| s.trim().parse().ok())))
    .filter(|p| (1..=65535).contains(p))
    .ok_or("invalid proxy port")?;
  let mut clean = obj.clone();
  clean.insert("server".into(), json!(server));
  clean.insert("port".into(), json!(port));
  let protocol = if obj.get("protocol").and_then(Value::as_str) == Some("http") { "http" } else { "socks5" };
  clean.insert("protocol".into(), json!(protocol));
  Ok(Value::Object(clean))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_tg_socks_link() {
    let p = parse_proxy_uri("tg://socks?server=1.2.3.4&port=1080&user=a&pass=b");
    assert_eq!(p["protocol"], "socks5");
    assert_eq!(p["server"], "1.2.3.4");
    assert_eq!(p["port"], 1080);
    assert_eq!(p["auth"]["username"], "a");
  }

  #[test]
  fn parses_socks_url_with_name() {
    let p = parse_proxy_uri("socks://u:p%40ss@example.com:9050#My%20Proxy");
    assert_eq!(p["name"], "My Proxy");
    assert_eq!(p["auth"]["password"], "p@ss");
    assert_eq!(p["port"], 9050);
  }

  #[test]
  fn parses_plain_host_port() {
    let p = parse_proxy_uri("example.com:1080");
    assert_eq!(p["protocol"], "socks5");
    assert!(p.get("rawLink").is_none());
  }

  #[test]
  fn rejects_garbage() {
    assert!(parse_proxy_uri("hello world").is_null());
    assert!(parse_proxy_uri("tg://socks?server=a;b&port=1").is_null());
  }
}
