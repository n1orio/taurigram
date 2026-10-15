// Протокол app:// — раздаёт собранный клиент (dist/) и служебные маршруты /ethernet/* (и алиасы /hermes/*).
// На Windows WebView2 видит его как http://app.localhost/.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::{Value, json};
use tauri::http::{Request, Response, StatusCode, header};

use crate::{paths, settings};

pub const SCHEME: &str = "app";

static LOADER_JS: &str = include_str!("../web/loader.js");
static NOTIFICATIONS_HTML: &str = include_str!("../web/notifications.html");

// Максимальный размер одного Range-ответа (видео-обои, звуки)
const MAX_RANGE_CHUNK: u64 = 8 * 1024 * 1024;

type Resp = Response<Vec<u8>>;

/// Базовый URL приложения для текущей платформы
pub fn base_url() -> url::Url {
  let raw = if cfg!(windows) { "http://app.localhost/" } else { "app://localhost/" };
  url::Url::parse(raw).unwrap()
}

pub fn is_app_url(url: &url::Url) -> bool {
  url.origin() == base_url().origin() || (url.scheme() == SCHEME && url.host_str() == Some("localhost"))
}

pub fn handle(request: &Request<Vec<u8>>) -> Resp {
  let uri = request.uri();
  let pathname = percent_decode(uri.path());
  let query = uri.query().unwrap_or("");

  match route(request, &pathname, query) {
    Ok(resp) => resp,
    Err(err) => {
      log::error!("[app protocol] {pathname}: {err}");
      text(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", "Internal error")
    }
  }
}

fn route(
  request: &Request<Vec<u8>>,
  pathname: &str,
  query: &str,
) -> Result<Resp, String> {
  if let Some(sub) = pathname.strip_prefix("/ethernet/").or_else(|| pathname.strip_prefix("/hermes/")) {
    return Ok(route_ethernet(request, sub, query));
  }
  Ok(route_dist(request, pathname))
}

fn route_ethernet(request: &Request<Vec<u8>>, sub: &str, query: &str) -> Resp {
  let p = paths::get();
  match sub {
    "loader.js" => text(StatusCode::OK, "application/javascript; charset=utf-8", LOADER_JS),
    "notifications.html" => text(StatusCode::OK, "text/html; charset=utf-8", NOTIFICATIONS_HTML),
    "themes.css" => {
      let mut css = String::new();
      for name in sorted_files(&p.themes, |f| f.ends_with(".css")) {
        if let Ok(content) = fs::read_to_string(p.themes.join(&name)) {
          css.push_str(&format!("/* theme: {name} */\n{content}\n"));
        }
      }
      text(StatusCode::OK, "text/css; charset=utf-8", &css)
    }
    "plugins/manifests.json" => {
      let safe_mode = settings::get_safe_mode();
      let list: Vec<Value> = list_plugin_dirs()
        .into_iter()
        .filter_map(|id| {
          let manifest = read_json(&p.plugins.join(&id).join("manifest.json"))?;
          Some(json!({
            "enabled": !safe_mode && settings::is_plugin_enabled(&id),
            "id": id,
            "name": manifest.get("name").cloned().unwrap_or(Value::Null),
          }))
        })
        .collect();
      json_resp(&Value::Array(list))
    }
    "config.json" => json_resp(&json!({ "theme": settings::get_theme() })),
    "mod.json" => {
      let mut m = settings::get_mod(None);
      if let (Some(wp), Some(obj)) = (settings::get_theme_wallpaper(None), m.as_object_mut()) {
        if let Some(file) = find_wallpaper_file(&wp) {
          let kind = wp.get("kind").and_then(Value::as_str).map(str::to_string)
            .unwrap_or_else(|| if is_video(&file) { "video".into() } else { "image".into() });
          obj.insert("wallpaperFile".into(), json!(file));
          obj.insert("wallpaperKind".into(), json!(kind));
        }
      }
      json_resp(&m)
    }
    "wallpaper.json" => {
      let theme = query_param(query, "theme");
      let empty = json!({ "slug": null, "file": null });
      let Some(wp) = settings::get_theme_wallpaper(theme.as_deref().or(settings::get_theme().as_deref())) else {
        return json_resp(&empty);
      };
      let Some(file) = find_wallpaper_file(&wp) else { return json_resp(&empty) };
      let video = wp.get("kind").and_then(Value::as_str) == Some("video") || is_video(&file);
      let ext = Path::new(&file).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
      let slug = wp.get("slug").filter(|s| s.as_str().is_some_and(|s| !s.is_empty())).cloned().unwrap_or(json!(file));
      json_resp(&json!({
        "slug": slug,
        "file": file,
        "kind": if video { "video" } else { "image" },
        "ext": ext,
        "originalPath": wp.get("originalPath").cloned().unwrap_or(Value::Null),
      }))
    }
    _ => {
      if let Some(name) = sub.strip_prefix("theme/").and_then(|n| n.strip_suffix(".css")) {
        let safe = base_name(name);
        let target = p.themes.join(format!("{safe}.css"));
        if !safe.is_empty() && paths::is_inside(&p.themes, &target) && target.is_file() {
          if let Ok(css) = fs::read_to_string(&target) {
            return text(StatusCode::OK, "text/css; charset=utf-8", &css);
          }
        }
        return not_found();
      }
      if let Some(file) = sub.strip_prefix("wallpapers/") {
        return serve_wallpaper(request, &base_name(file));
      }
      if let Some(rest) = sub.strip_prefix("plugins/") {
        if let Some((id, file)) = rest.split_once('/') {
          let valid_id = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
          if valid_id && (file == "manifest.json" || file == "index.js") {
            if let Ok(content) = fs::read(p.plugins.join(id).join(file)) {
              let mime = if file.ends_with(".js") {
                "application/javascript; charset=utf-8"
              } else {
                "application/json; charset=utf-8"
              };
              return bytes(StatusCode::OK, mime, content);
            }
          }
        }
      }
      not_found()
    }
  }
}

fn route_dist(request: &Request<Vec<u8>>, pathname: &str) -> Resp {
  let dist = &paths::get().dist;
  let rel = pathname.trim_start_matches('/');

  if !rel.is_empty() {
    let target = dist.join(rel);
    if !paths::is_inside(dist, &target) {
      return text(StatusCode::FORBIDDEN, "text/plain", "Forbidden");
    }
    if target.is_file() && !rel.ends_with("index.html") {
      return serve_file(request, &target);
    }
    // Медиа, прогрессивные потоки, ассеты и файлы с расширением — 404
    let has_ext = Path::new(rel).extension().is_some();
    if rel.starts_with("progressive/") || rel.starts_with("download/") || rel.starts_with("assets/") || has_ext {
      return not_found();
    }
  }

  // index.html (fallback для SPA-роутинга) с инжектом лоадера
  let Ok(mut html) = fs::read_to_string(dist.join("index.html")) else {
    return text(StatusCode::NOT_FOUND, "text/plain", "dist not built — run: npm run app:build");
  };
  let inject = r#"<script src="/ethernet/loader.js"></script>"#;
  if !html.contains(inject) {
    html = html.replacen("</head>", &format!("{inject}\n</head>"), 1);
  }
  let mut resp = text(StatusCode::OK, "text/html; charset=utf-8", &html);
  let headers = resp.headers_mut();
  headers.insert("x-content-type-options", "nosniff".parse().unwrap());
  headers.insert("x-frame-options", "SAMEORIGIN".parse().unwrap());
  resp
}

fn serve_wallpaper(request: &Request<Vec<u8>>, file: &str) -> Resp {
  let dir = &paths::get().wallpapers;
  let target = dir.join(file);
  if file.is_empty() || !paths::is_inside(dir, &target) || !target.is_file() {
    return not_found();
  }
  serve_file(request, &target)
}

/// Отдаёт файл с диска с поддержкой Range-запросов (нужно для видео/аудио)
fn serve_file(request: &Request<Vec<u8>>, target: &Path) -> Resp {
  let mime = mime_of(target);
  let Ok(mut f) = fs::File::open(target) else { return not_found() };
  let total = f.metadata().map(|m| m.len()).unwrap_or(0);

  let range = request
    .headers()
    .get(header::RANGE)
    .and_then(|v| v.to_str().ok())
    .and_then(parse_range);

  if let Some((start, end)) = range {
    let end = end.unwrap_or(total.saturating_sub(1)).min(total.saturating_sub(1));
    if start < total && end >= start {
      let end = end.min(start + MAX_RANGE_CHUNK - 1);
      let len = end - start + 1;
      let mut buf = vec![0u8; len as usize];
      if f.seek(SeekFrom::Start(start)).is_ok() && f.read_exact(&mut buf).is_ok() {
        return Response::builder()
          .status(StatusCode::PARTIAL_CONTENT)
          .header(header::CONTENT_TYPE, mime)
          .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
          .header(header::ACCEPT_RANGES, "bytes")
          .header(header::CONTENT_LENGTH, len.to_string())
          .body(buf)
          .unwrap();
      }
    }
  }

  let mut buf = Vec::with_capacity(total as usize);
  if f.read_to_end(&mut buf).is_err() {
    return not_found();
  }
  Response::builder()
    .status(StatusCode::OK)
    .header(header::CONTENT_TYPE, mime)
    .header(header::ACCEPT_RANGES, "bytes")
    .header(header::CONTENT_LENGTH, buf.len().to_string())
    .body(buf)
    .unwrap()
}

fn mime_of(path: &Path) -> &'static str {
  let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
  match ext.as_str() {
    "html" | "htm" => "text/html; charset=utf-8",
    "js" | "mjs" => "application/javascript; charset=utf-8",
    "css" => "text/css; charset=utf-8",
    "json" | "map" => "application/json; charset=utf-8",
    "webmanifest" => "application/manifest+json",
    "wasm" => "application/wasm",
    "txt" => "text/plain; charset=utf-8",
    "xml" => "application/xml",
    "svg" => "image/svg+xml",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "ico" => "image/x-icon",
    "mp4" => "video/mp4",
    "webm" => "video/webm",
    "mp3" => "audio/mpeg",
    "ogg" | "oga" => "audio/ogg",
    "wav" => "audio/wav",
    "woff" => "font/woff",
    "woff2" => "font/woff2",
    "ttf" => "font/ttf",
    "otf" => "font/otf",
    _ => "application/octet-stream",
  }
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
  let spec = value.strip_prefix("bytes=")?;
  let (start, end) = spec.split_once('-')?;
  let start = start.trim().parse().ok()?;
  let end = end.trim();
  Some((start, if end.is_empty() { None } else { end.parse().ok() }))
}

// --- Общие помощники ---

pub fn is_video(file: &str) -> bool {
  let lower = file.to_lowercase();
  lower.ends_with(".mp4") || lower.ends_with(".webm")
}

/// Поиск файла обоев в wallpapers/ по описанию из настроек
pub fn find_wallpaper_file(wp: &Value) -> Option<String> {
  let wp_file = wp.get("file").and_then(Value::as_str).filter(|s| !s.is_empty());
  let wp_slug = wp.get("slug").and_then(Value::as_str).filter(|s| !s.is_empty());
  if wp_file.is_none() && wp_slug.is_none() {
    return None;
  }
  let files = sorted_files(&paths::get().wallpapers, |_| true);
  if let Some(file) = wp_file {
    if files.iter().any(|f| f == file) {
      return Some(file.to_string());
    }
  }
  let prefix = wp_file.or(wp_slug).unwrap();
  files
    .into_iter()
    .find(|f| f.starts_with(prefix) || wp_slug.is_some_and(|slug| f.starts_with(slug)))
}

pub fn sorted_files(dir: &Path, filter: impl Fn(&str) -> bool) -> Vec<String> {
  let mut names: Vec<String> = fs::read_dir(dir)
    .map(|rd| {
      rd.flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| filter(n))
        .collect()
    })
    .unwrap_or_default();
  names.sort();
  names
}

pub fn list_plugin_dirs() -> Vec<String> {
  let mut ids: Vec<String> = fs::read_dir(&paths::get().plugins)
    .map(|rd| {
      rd.flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
    })
    .unwrap_or_default();
  ids.sort();
  ids
}

pub fn read_json(path: &Path) -> Option<Value> {
  serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Аналог `path.basename` — отбрасывает любые каталоги
pub fn base_name(name: &str) -> String {
  name.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

fn query_param(query: &str, key: &str) -> Option<String> {
  url::form_urlencoded::parse(query.as_bytes())
    .find(|(k, _)| k == key)
    .map(|(_, v)| v.into_owned())
    .filter(|v| !v.is_empty())
}

fn percent_decode(path: &str) -> String {
  url::form_urlencoded::parse(format!("p={}", path.replace('+', "%2B")).as_bytes())
    .next()
    .map(|(_, v)| v.into_owned())
    .unwrap_or_else(|| path.to_string())
}

fn bytes(status: StatusCode, mime: &str, body: Vec<u8>) -> Resp {
  Response::builder()
    .status(status)
    .header(header::CONTENT_TYPE, mime)
    .body(body)
    .unwrap()
}

fn text(status: StatusCode, mime: &str, body: &str) -> Resp {
  bytes(status, mime, body.as_bytes().to_vec())
}

fn json_resp(v: &Value) -> Resp {
  text(StatusCode::OK, "application/json; charset=utf-8", &v.to_string())
}

fn not_found() -> Resp {
  text(StatusCode::NOT_FOUND, "text/plain", "Not Found")
}
