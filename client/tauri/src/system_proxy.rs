// Системный прокси ОС для режима «без своего прокси».
//
// Источники: переменные окружения (`ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`),
// Windows — реестр Internet Settings, Linux — GNOME (gsettings) и KDE (kioslaverc),
// macOS — `scutil --proxy`. PAC-скрипты не поддерживаются.
// Результат кешируется на `CACHE_TTL`, чтобы не опрашивать систему на каждое соединение.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
  Http,
  Socks5,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
  pub protocol: Protocol,
  pub server: String,
  pub port: u16,
  pub username: String,
  pub password: String,
}

const CACHE_TTL: Duration = Duration::from_secs(30);

static CACHE: Mutex<Option<(Instant, Option<Upstream>)>> = Mutex::new(None);

pub fn get() -> Option<Upstream> {
  let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
  if let Some((at, value)) = cache.as_ref() {
    if at.elapsed() < CACHE_TTL {
      return value.clone();
    }
  }
  let value = detect();
  *cache = Some((Instant::now(), value.clone()));
  value
}

/// Сбрасывает кеш (после смены сетевых настроек)
pub fn invalidate() {
  *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn detect() -> Option<Upstream> {
  from_env().or_else(from_os)
}

fn from_env() -> Option<Upstream> {
  ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
    .iter()
    .filter_map(|k| std::env::var(k).ok())
    .find_map(|v| parse_proxy_url(&v, Protocol::Http))
}

/// `socks5://user:pass@host:port`, `http://host:port` или просто `host:port`
pub fn parse_proxy_url(raw: &str, default_protocol: Protocol) -> Option<Upstream> {
  let raw = raw.trim();
  if raw.is_empty() {
    return None;
  }
  let with_scheme = if raw.contains("://") {
    raw.to_string()
  } else {
    let scheme = if default_protocol == Protocol::Socks5 { "socks5" } else { "http" };
    format!("{scheme}://{raw}")
  };
  let url = url::Url::parse(&with_scheme).ok()?;
  let protocol = match url.scheme() {
    "socks" | "socks5" | "socks5h" | "socks4" | "socks4a" => Protocol::Socks5,
    "http" | "https" => Protocol::Http,
    _ => return None,
  };
  let server = url.host_str()?.trim_matches(['[', ']']).to_string();
  let port = url.port().unwrap_or(match protocol {
    Protocol::Socks5 => 1080,
    Protocol::Http => 8080,
  });
  Some(Upstream {
    protocol,
    server,
    port,
    username: url.username().to_string(),
    password: url.password().unwrap_or_default().to_string(),
  })
}

/// Строка формата Windows: `host:port` или `http=h:p;https=h:p;socks=h:p`
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_windows_proxy_server(value: &str) -> Option<Upstream> {
  if !value.contains('=') {
    return parse_proxy_url(value, Protocol::Http);
  }
  let entries: Vec<(String, String)> = value
    .split(';')
    .filter_map(|part| part.split_once('='))
    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
    .collect();
  let find = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
  find("socks")
    .and_then(|v| parse_proxy_url(v, Protocol::Socks5))
    .or_else(|| find("https").and_then(|v| parse_proxy_url(v, Protocol::Http)))
    .or_else(|| find("http").and_then(|v| parse_proxy_url(v, Protocol::Http)))
}

#[cfg(windows)]
fn from_os() -> Option<Upstream> {
  use winreg::RegKey;
  use winreg::enums::HKEY_CURRENT_USER;

  let key = RegKey::predef(HKEY_CURRENT_USER)
    .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
    .ok()?;
  let enabled: u32 = key.get_value("ProxyEnable").ok()?;
  if enabled == 0 {
    return None;
  }
  let server: String = key.get_value("ProxyServer").ok()?;
  parse_windows_proxy_server(&server)
}

#[cfg(target_os = "linux")]
fn from_os() -> Option<Upstream> {
  from_gnome().or_else(from_kde)
}

#[cfg(target_os = "linux")]
fn gsettings(schema: &str, key: &str) -> Option<String> {
  let out = std::process::Command::new("gsettings").args(["get", schema, key]).output().ok()?;
  if !out.status.success() {
    return None;
  }
  Some(String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string())
}

#[cfg(target_os = "linux")]
fn from_gnome() -> Option<Upstream> {
  if gsettings("org.gnome.system.proxy", "mode")? != "manual" {
    return None;
  }
  [("socks", Protocol::Socks5), ("https", Protocol::Http), ("http", Protocol::Http)]
    .into_iter()
    .find_map(|(kind, protocol)| {
      let schema = format!("org.gnome.system.proxy.{kind}");
      let host = gsettings(&schema, "host").filter(|h| !h.is_empty())?;
      let port: u16 = gsettings(&schema, "port")?.parse().ok().filter(|p| *p > 0)?;
      Some(Upstream { protocol, server: host, port, username: String::new(), password: String::new() })
    })
}

#[cfg(target_os = "linux")]
fn from_kde() -> Option<Upstream> {
  let config_dir = std::env::var_os("XDG_CONFIG_HOME")
    .map(std::path::PathBuf::from)
    .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
  let content = std::fs::read_to_string(config_dir.join("kioslaverc")).ok()?;
  parse_kioslaverc(&content)
}

/// `[Proxy Settings]` из kioslaverc: `ProxyType=1` — ручная настройка, адреса вида `socks://host port`
pub fn parse_kioslaverc(content: &str) -> Option<Upstream> {
  let mut in_section = false;
  let mut values = std::collections::HashMap::new();
  for line in content.lines() {
    let line = line.trim();
    if line.starts_with('[') {
      in_section = line == "[Proxy Settings]";
      continue;
    }
    if in_section {
      if let Some((k, v)) = line.split_once('=') {
        values.insert(k.trim().to_string(), v.trim().to_string());
      }
    }
  }
  if values.get("ProxyType").map(String::as_str) != Some("1") {
    return None;
  }
  [("socksProxy", Protocol::Socks5), ("httpsProxy", Protocol::Http), ("httpProxy", Protocol::Http)]
    .into_iter()
    .find_map(|(key, protocol)| {
      let raw = values.get(key)?.replace(' ', ":");
      parse_proxy_url(&raw, protocol)
    })
}

#[cfg(target_os = "macos")]
fn from_os() -> Option<Upstream> {
  let out = std::process::Command::new("scutil").arg("--proxy").output().ok()?;
  let text = String::from_utf8_lossy(&out.stdout);
  let value = |key: &str| {
    text.lines().find_map(|l| {
      let (k, v) = l.split_once(':')?;
      (k.trim() == key).then(|| v.trim().to_string())
    })
  };
  [("SOCKS", Protocol::Socks5), ("HTTPS", Protocol::Http), ("HTTP", Protocol::Http)]
    .into_iter()
    .find_map(|(kind, protocol)| {
      if value(&format!("{kind}Enable"))? != "1" {
        return None;
      }
      let server = value(&format!("{kind}Proxy"))?;
      let port = value(&format!("{kind}Port"))?.parse().ok()?;
      Some(Upstream { protocol, server, port, username: String::new(), password: String::new() })
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn from_os() -> Option<Upstream> {
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_windows_formats() {
    let p = parse_windows_proxy_server("127.0.0.1:8888").unwrap();
    assert_eq!((p.protocol, p.server.as_str(), p.port), (Protocol::Http, "127.0.0.1", 8888));
    let p = parse_windows_proxy_server("http=a:1;https=b:2;socks=c:3").unwrap();
    assert_eq!((p.protocol, p.server.as_str(), p.port), (Protocol::Socks5, "c", 3));
    let p = parse_windows_proxy_server("http=a:1;https=b:2").unwrap();
    assert_eq!((p.server.as_str(), p.port), ("b", 2));
  }

  #[test]
  fn parses_env_urls() {
    let p = parse_proxy_url("socks5h://u:p@10.0.0.1:1081", Protocol::Http).unwrap();
    assert_eq!((p.protocol, p.port, p.username.as_str()), (Protocol::Socks5, 1081, "u"));
    assert!(parse_proxy_url("", Protocol::Http).is_none());
  }

  #[test]
  fn parses_kde_config() {
    let cfg = "[Proxy Settings]\nProxyType=1\nsocksProxy=socks://127.0.0.1 1080\n";
    let p = parse_kioslaverc(cfg).unwrap();
    assert_eq!((p.protocol, p.server.as_str(), p.port), (Protocol::Socks5, "127.0.0.1", 1080));
    assert!(parse_kioslaverc("[Proxy Settings]\nProxyType=0\n").is_none());
  }
}
