// Команды window.ethernetDesktop (бывший Electron preload + ipcMain-хендлеры).
// Файловые операции с темами/плагинами/обоями выполняются только здесь — у страницы нет доступа к ФС.

use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde_json::{Value, json};
use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::protocol::{base_name, is_video, list_plugin_dirs, read_json, sorted_files};
use crate::{notifications, paths, proxy, settings, tray};

type CmdResult<T = Value> = Result<T, String>;

const MEDIA_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "gif", "mp4", "webm"];
const MAX_PICKED_BINARY_SIZE: u64 = 20 * 1024 * 1024;
const MAX_WALLPAPER_BASE64_SIZE: usize = 500 * 1024 * 1024;
const MAX_ALLOWED_PICKED_PATHS: usize = 50;

// Пути, выбранные пользователем в системном диалоге (только их можно копировать в обои)
static ALLOWED_PICKED_PATHS: Mutex<VecDeque<PathBuf>> = Mutex::new(VecDeque::new());

fn err(e: impl ToString) -> String {
  e.to_string()
}

/// Аналог safeName из Electron-версии: имя файла без каталогов и опасных символов
fn safe_name(name: &str, ext: &str) -> String {
  let base = base_name(name);
  let stem = Path::new(&base).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
  let stem = if Path::new(&base).extension().is_some() { stem } else { base.clone() };
  let mut cleaned = String::new();
  let mut in_bad_run = false;
  for c in stem.chars() {
    let is_ok = c.is_ascii_alphanumeric() || c == '_' || c == ' ' || c == '-' || ('\u{0400}'..='\u{04FF}').contains(&c);
    if is_ok {
      cleaned.push(c);
      in_bad_run = false;
    } else if !in_bad_run {
      cleaned.push('-');
      in_bad_run = true;
    }
  }
  let trimmed: String = cleaned.trim_matches('-').chars().take(64).collect();
  let base = if trimmed.is_empty() { "untitled".to_string() } else { trimmed };
  base + ext
}

/// Имя темы без `.css` и спецсимволов
fn clean_theme_base(name: &str) -> CmdResult<String> {
  let base = base_name(name);
  let base = base.strip_suffix(".css").unwrap_or(&base);
  let clean: String = base
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ' ' || *c == '-' || ('\u{0400}'..='\u{04FF}').contains(c))
    .collect();
  if clean.is_empty() { Err("bad name".into()) } else { Ok(clean) }
}

fn theme_path(base: &str) -> CmdResult<PathBuf> {
  let dir = &paths::get().themes;
  let target = dir.join(format!("{base}.css"));
  if !paths::is_inside(dir, &target) {
    return Err("bad path".into());
  }
  Ok(target)
}

fn plugin_dir(id: &str) -> CmdResult<PathBuf> {
  let is_valid = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
  if !is_valid {
    return Err("bad id".into());
  }
  let dir = &paths::get().plugins;
  let target = dir.join(id);
  if !paths::is_inside(dir, &target) {
    return Err("bad path".into());
  }
  Ok(target)
}

// --- Темы ---

#[tauri::command]
pub fn ethernet_themes_list() -> Value {
  paths::ensure_dirs();
  let active = settings::get_theme();
  let list: Vec<Value> = sorted_files(&paths::get().themes, |f| f.ends_with(".css"))
    .into_iter()
    .map(|f| {
      let is_active = active.as_deref() == Some(f.trim_end_matches(".css"));
      json!({ "name": f, "active": is_active })
    })
    .collect();
  Value::Array(list)
}

#[tauri::command]
pub fn ethernet_theme_read(name: String) -> CmdResult<String> {
  let target = theme_path(&clean_theme_base(&name)?)?;
  if !target.is_file() {
    return Err("theme not found".into());
  }
  fs::read_to_string(target).map_err(err)
}

#[tauri::command]
pub fn ethernet_theme_save(name: String, css: Option<String>, wallpaper_info: Option<Value>) -> CmdResult<String> {
  let safe = safe_name(&name, ".css");
  let target = paths::get().themes.join(&safe);
  if !paths::is_inside(&paths::get().themes, &target) {
    return Err("bad path".into());
  }
  fs::write(&target, css.unwrap_or_default()).map_err(err)?;
  let clean = safe.trim_end_matches(".css").to_string();
  match wallpaper_info.filter(|w| !w.is_null()) {
    Some(info) => {
      settings::set_theme_wallpaper(Some(&clean), Some(&info));
    }
    None => {
      if let Some(current) = settings::get_theme_wallpaper(None) {
        settings::set_theme_wallpaper(Some(&clean), Some(&current));
      }
    }
  }
  Ok(safe)
}

#[tauri::command]
pub fn ethernet_theme_delete(name: String) -> CmdResult<bool> {
  let base = clean_theme_base(&name)?;
  let target = theme_path(&base)?;
  if target.exists() {
    fs::remove_file(target).map_err(err)?;
  }
  settings::set_theme_wallpaper(Some(&base), None);
  Ok(true)
}

#[tauri::command]
pub fn ethernet_theme_activate(name: Option<String>) -> CmdResult {
  // name = "amoled" (без .css) или null, чтобы сбросить
  let Some(name) = name else {
    settings::set_theme(None);
    return Ok(json!({ "theme": null, "wallpaper": null }));
  };
  let clean = base_name(&name);
  if clean.contains("..") {
    return Err("bad name".into());
  }
  let theme = if clean.is_empty() { None } else { Some(clean.clone()) };
  settings::set_theme(theme.clone());
  let wallpaper = settings::get_theme_wallpaper(theme.as_deref());
  Ok(json!({ "theme": theme, "wallpaper": wallpaper }))
}

// --- Плагины ---

#[tauri::command]
pub fn ethernet_plugins_list() -> Value {
  paths::ensure_dirs();
  let list: Vec<Value> = list_plugin_dirs()
    .into_iter()
    .map(|id| {
      let manifest = read_json(&paths::get().plugins.join(&id).join("manifest.json")).unwrap_or_else(|| json!({}));
      let field = |k: &str| manifest.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
      let name = Some(field("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
      json!({
        "id": id,
        "name": name,
        "description": field("description"),
        "version": field("version"),
        "enabled": settings::is_plugin_enabled(&id),
      })
    })
    .collect();
  Value::Array(list)
}

#[tauri::command]
pub fn ethernet_plugin_read(id: String) -> CmdResult {
  let dir = plugin_dir(&id)?;
  let read = |f: &str| fs::read_to_string(dir.join(f)).unwrap_or_default();
  let manifest = Some(read("manifest.json")).filter(|m| !m.is_empty()).unwrap_or_else(|| "{}".into());
  Ok(json!({ "manifest": manifest, "code": read("index.js") }))
}

/// Сохранение плагина: { name, description, version, code } -> plugins/<id>/{manifest.json,index.js}
#[tauri::command]
pub fn ethernet_plugin_save(plugin: Value) -> CmdResult<String> {
  if !plugin.is_object() {
    return Err("bad plugin".into());
  }
  let text = |k: &str| plugin.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
  let raw_name = Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| "plugin".into());
  let raw_id = safe_name(&raw_name, "").to_lowercase();
  let mut id = String::new();
  let mut in_bad_run = false;
  for c in raw_id.chars() {
    if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
      id.push(c);
      in_bad_run = false;
    } else if !in_bad_run {
      id.push('-');
      in_bad_run = true;
    }
  }
  let id: String = id.trim_matches('-').chars().take(48).collect();
  let dir = plugin_dir(&id).map_err(|_| "bad name".to_string())?;
  fs::create_dir_all(&dir).map_err(err)?;

  let truncate = |s: String, n: usize| s.chars().take(n).collect::<String>();
  let name = Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
  let version = Some(text("version")).filter(|v| !v.is_empty()).unwrap_or_else(|| "0.1.0".into());
  let manifest = json!({
    "name": truncate(name, 64),
    "description": truncate(text("description"), 256),
    "version": truncate(version, 16),
    "author": "user",
  });
  fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).map_err(err)?).map_err(err)?;
  fs::write(dir.join("index.js"), text("code")).map_err(err)?;
  Ok(id)
}

#[tauri::command]
pub fn ethernet_plugin_delete(id: String) -> CmdResult<bool> {
  let dir = plugin_dir(&id)?;
  if dir.exists() {
    fs::remove_dir_all(dir).map_err(err)?;
  }
  settings::remove_plugin(&id);
  Ok(true)
}

#[tauri::command]
pub fn ethernet_plugin_toggle(id: String) -> CmdResult<bool> {
  plugin_dir(&id)?;
  Ok(settings::toggle_plugin(&id))
}

// --- Диалоги выбора / сохранения файла ---

#[tauri::command]
pub async fn ethernet_pick_file(app: AppHandle, kind: Option<String>) -> CmdResult {
  let kind = kind.unwrap_or_default();
  let (filter_name, extensions, is_binary): (&str, Vec<&str>, bool) = match kind.as_str() {
    "css" => ("CSS Theme", vec!["css"], false),
    "wallpaper" | "media" | "image" | "video" => ("Images and Videos", MEDIA_EXTENSIONS.to_vec(), true),
    "json" | "theme-json" => ("JSON Theme", vec!["json"], false),
    "js" | "plugin" => ("Plugin (JS)", vec!["js"], false),
    _ => ("All Files", vec!["*"], false),
  };

  let picked = app.dialog().file().add_filter(filter_name, &extensions).blocking_pick_file();
  let Some(file) = picked.and_then(|f| f.into_path().ok()) else { return Ok(Value::Null) };

  {
    let resolved = file.canonicalize().unwrap_or_else(|_| file.clone());
    let mut allowed = ALLOWED_PICKED_PATHS.lock().unwrap_or_else(|e| e.into_inner());
    allowed.push_back(resolved);
    while allowed.len() > MAX_ALLOWED_PICKED_PATHS {
      allowed.pop_front();
    }
  }

  let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  let path = file.to_string_lossy().into_owned();
  let content = if is_binary {
    match fs::metadata(&file) {
      Ok(meta) if meta.len() <= MAX_PICKED_BINARY_SIZE => {
        fs::read(&file).map(|b| base64::engine::general_purpose::STANDARD.encode(b)).unwrap_or_default()
      }
      _ => String::new(),
    }
  } else {
    fs::read_to_string(&file).map_err(err)?
  };
  Ok(json!({ "name": name, "path": path, "content": content }))
}

#[tauri::command]
pub async fn ethernet_save_file(app: AppHandle, payload: Value) -> CmdResult<bool> {
  let text = |k: &str, default: &str| {
    payload.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(default).to_string()
  };
  let ext = text("ext", "css");
  let target = app
    .dialog()
    .file()
    .set_file_name(text("defaultName", "theme.css"))
    .add_filter(text("filterName", "CSS Theme"), &[ext.as_str()])
    .blocking_save_file();
  let Some(path) = target.and_then(|f| f.into_path().ok()) else { return Ok(false) };
  fs::write(path, text("content", "")).map_err(err)?;
  Ok(true)
}

// --- Окно ---

#[tauri::command]
pub fn ethernet_reload(window: WebviewWindow) -> bool {
  window.reload().is_ok()
}

#[tauri::command]
pub fn ethernet_toggle_devtools(window: WebviewWindow) {
  if window.is_devtools_open() {
    window.close_devtools();
  } else {
    window.open_devtools();
  }
}

#[tauri::command]
pub fn ethernet_window_minimize(window: WebviewWindow) -> bool {
  window.minimize().is_ok()
}

#[tauri::command]
pub fn ethernet_window_maximize_toggle(window: WebviewWindow) -> bool {
  if window.is_maximized().unwrap_or(false) {
    let _ = window.unmaximize();
    false
  } else {
    let _ = window.maximize();
    true
  }
}

#[tauri::command]
pub fn ethernet_window_close(window: WebviewWindow) -> bool {
  window.close().is_ok()
}

#[tauri::command]
pub fn ethernet_window_is_maximized(window: WebviewWindow) -> bool {
  window.is_maximized().unwrap_or(false)
}

// --- Настройки мода (скругления, блюр, анимации) ---

#[tauri::command]
pub fn ethernet_mod_get() -> Value {
  settings::get_mod(None)
}

#[tauri::command]
pub fn ethernet_mod_set(r#mod: Value) -> Value {
  settings::set_mod(&r#mod, None)
}

// --- Ethernet-обои: медиафайлы в wallpapers/ ---

fn is_safe_media(buf: &[u8]) -> bool {
  if buf.len() < 4 {
    return false;
  }
  buf.starts_with(&[0x89, 0x50, 0x4e, 0x47]) // PNG
    || buf.starts_with(&[0xff, 0xd8, 0xff]) // JPEG
    || buf.starts_with(b"GIF8") // GIF
    || (buf.len() >= 12 && &buf[0..4] == b"RIFF" && &buf[8..12] == b"WEBP") // WEBP
    || (buf.len() >= 8 && &buf[4..8] == b"ftyp") // MP4
    || buf.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) // WEBM / MKV
}

fn extension_of(name: &str) -> Option<String> {
  Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase())
}

fn clear_wallpapers_dir() {
  let dir = &paths::get().wallpapers;
  for name in sorted_files(dir, |_| true) {
    let target = dir.join(&name);
    if paths::is_inside(dir, &target) {
      let _ = fs::remove_file(target);
    }
  }
}

#[tauri::command]
pub fn ethernet_wallpaper_set_file(payload: Value) -> CmdResult {
  // payload = { name, content/base64, originalPath, path, themeName }
  if !payload.is_object() {
    return Err("bad payload".into());
  }
  let get = |k: &str| payload.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);

  let mut source = get("path").or_else(|| get("originalPath")).map(PathBuf::from).filter(|p| p.is_file());
  // Локальный файл принимается, только если пользователь сам выбрал его в системном диалоге
  if let Some(src) = &source {
    let resolved = src.canonicalize().unwrap_or_else(|_| src.clone());
    let allowed = ALLOWED_PICKED_PATHS.lock().unwrap_or_else(|e| e.into_inner());
    if !allowed.contains(&resolved) {
      log::warn!("[Security] Ignored unverified local file path for wallpaper: {}", src.display());
      source = None;
    }
  }

  let raw_data = get("base64").or_else(|| get("content"));
  if source.is_none() && raw_data.is_none() {
    return Err("bad media content".into());
  }
  if raw_data.as_ref().is_some_and(|d| d.len() > MAX_WALLPAPER_BASE64_SIZE) {
    return Err("media too large".into());
  }

  let ext_source = get("name").or_else(|| source.as_ref().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
  let ext = extension_of(&ext_source).unwrap_or_else(|| "png".into());
  if !MEDIA_EXTENSIONS.contains(&ext.as_str()) {
    return Err("bad media type".into());
  }

  let media = match &source {
    Some(src) => {
      let src_ext = extension_of(&src.to_string_lossy()).unwrap_or_default();
      if !MEDIA_EXTENSIONS.contains(&src_ext.as_str()) {
        return Err("bad source media extension".into());
      }
      let mut header = [0u8; 16];
      let n = fs::File::open(src).and_then(|mut f| f.read(&mut header)).map_err(err)?;
      if !is_safe_media(&header[..n]) {
        return Err("invalid media file header".into());
      }
      None
    }
    None => {
      let data = base64::engine::general_purpose::STANDARD
        .decode(raw_data.unwrap().trim())
        .map_err(|_| "invalid media data".to_string())?;
      if !is_safe_media(&data) {
        return Err("invalid media data".into());
      }
      Some(data)
    }
  };

  let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
  let slug = format!("ethernet-{millis}");
  let filename = format!("{slug}.{ext}");
  let dir = &paths::get().wallpapers;
  let target = dir.join(&filename);
  if !paths::is_inside(dir, &target) {
    return Err("bad path".into());
  }

  // Храним только последние выбранные обои
  clear_wallpapers_dir();
  match (&source, media) {
    (Some(src), _) => {
      fs::copy(src, &target).map_err(err)?;
    }
    (None, Some(data)) => fs::write(&target, data).map_err(err)?,
    (None, None) => unreachable!(),
  }

  let original: String = source
    .as_ref()
    .map(|s| s.to_string_lossy().into_owned())
    .or_else(|| get("name"))
    .unwrap_or_default()
    .chars()
    .take(256)
    .collect();
  let info = json!({
    "slug": slug,
    "file": filename,
    "kind": if is_video(&filename) { "video" } else { "image" },
    "originalPath": original,
  });
  let theme = get("themeName").or_else(settings::get_theme).unwrap_or_else(|| "default".into());
  settings::set_theme_wallpaper(Some(&theme), Some(&info));
  Ok(info)
}

#[tauri::command]
pub fn ethernet_wallpaper_clear(theme_name: Option<String>) -> Value {
  let theme = theme_name.filter(|t| !t.is_empty()).or_else(settings::get_theme).unwrap_or_else(|| "default".into());
  settings::set_theme_wallpaper(Some(&theme), None);
  clear_wallpapers_dir();
  json!({ "slug": null, "file": null })
}

// --- Сеть и прокси ---

#[tauri::command]
pub async fn ethernet_network_geo_info() -> Value {
  proxy::network_geo_info().await
}

#[tauri::command]
pub fn ethernet_proxy_get_state() -> Value {
  settings::get_proxy_settings()
}

#[tauri::command]
pub fn ethernet_proxy_save(app: AppHandle, proxy: Value) -> CmdResult {
  let clean = proxy::clean_proxy_config(&proxy)?;
  let state = settings::save_proxy(clean);
  proxy::apply_network_state(&app);
  Ok(state)
}

#[tauri::command]
pub fn ethernet_proxy_delete(app: AppHandle, id: Value) -> Value {
  let state = settings::delete_proxy(&id);
  proxy::apply_network_state(&app);
  state
}

#[tauri::command]
pub fn ethernet_proxy_set_active(app: AppHandle, id: Value) -> Value {
  let state = settings::set_active_proxy(id);
  proxy::apply_network_state(&app);
  state
}

#[tauri::command]
pub fn ethernet_proxy_toggle(app: AppHandle, enabled: Option<bool>) -> Value {
  let state = settings::toggle_proxy(enabled);
  proxy::apply_network_state(&app);
  state
}

#[tauri::command]
pub fn ethernet_proxy_parse_link(link: Option<String>) -> Value {
  link.map(|l| proxy::parse_proxy_uri(&l)).unwrap_or(Value::Null)
}

#[tauri::command]
pub async fn ethernet_proxy_test_ping(proxy: Value) -> i64 {
  proxy::test_ping(&proxy).await
}

// --- Уведомления ---

#[tauri::command]
pub fn ethernet_show_notification(app: AppHandle, payload: Value) -> bool {
  notifications::show(&app, payload);
  true
}

#[tauri::command]
pub fn ethernet_notifications_disabled_get() -> bool {
  settings::get_notifications_disabled()
}

#[tauri::command]
pub fn ethernet_notifications_disabled_set(disabled: bool) -> bool {
  let result = settings::set_notifications_disabled(disabled);
  tray::sync_notifications_item(result);
  result
}

#[tauri::command]
pub fn ethernet_close_to_tray_disabled_get() -> bool {
  settings::get_close_to_tray_disabled()
}

#[tauri::command]
pub fn ethernet_close_to_tray_disabled_set(disabled: bool) -> bool {
  settings::set_close_to_tray_disabled(disabled)
}

// --- Безопасный режим (Safe Mode) ---

#[tauri::command]
pub fn ethernet_safemode_get() -> bool {
  settings::get_safe_mode()
}

#[tauri::command]
pub fn ethernet_safemode_set(enabled: bool) -> bool {
  settings::set_safe_mode(enabled)
}

#[tauri::command]
pub fn ethernet_last_crashed_plugin_get() -> Value {
  settings::get_last_crashed_plugin()
}

#[tauri::command]
pub fn ethernet_clear_last_crashed_plugin() -> bool {
  settings::clear_last_crashed_plugin();
  true
}

#[tauri::command]
pub fn ethernet_report_plugin_crash(plugin_id: String, error: Option<String>) -> Value {
  settings::report_plugin_crash(&plugin_id, error.as_deref().unwrap_or(""))
}

// --- Счётчик непрочитанных для панели задач / трея ---

#[tauri::command]
pub fn ethernet_set_unread_count(app: AppHandle, count: Option<i32>) -> bool {
  tray::set_unread_count(&app, count.unwrap_or(0), false);
  true
}

// --- Окно уведомлений ---

fn ensure_notif_window(window: &WebviewWindow) -> CmdResult<()> {
  if window.label() == notifications::LABEL { Ok(()) } else { Err("Unauthorized".into()) }
}

#[tauri::command]
pub fn notif_ready(app: AppHandle, window: WebviewWindow) -> CmdResult<()> {
  ensure_notif_window(&window)?;
  notifications::mark_ready(&app);
  Ok(())
}

#[tauri::command]
pub fn notif_clicked(app: AppHandle, window: WebviewWindow, data: Value) -> CmdResult<()> {
  ensure_notif_window(&window)?;
  notifications::handle_click(&app, data);
  Ok(())
}

#[tauri::command]
pub fn notif_dismissed(window: WebviewWindow) -> CmdResult<()> {
  ensure_notif_window(&window)
}

#[tauri::command]
pub fn notif_set_height(app: AppHandle, window: WebviewWindow, height: f64) -> CmdResult<()> {
  ensure_notif_window(&window)?;
  notifications::set_height(&app, height);
  Ok(())
}

// --- Экспорт сессии в Telegram Desktop (tdata) ---

/// Создаёт папку `tdata` с текущей сессией в выбранном пользователем каталоге.
/// `session` = { userId, mainDcId, keys: { [dcId]: hex } }. Возвращает путь или `null` при отмене.
#[tauri::command]
pub async fn ethernet_export_tdata(app: AppHandle, session: Value) -> CmdResult {
  let user_id: u64 = session
    .get("userId")
    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    .filter(|id| *id > 0)
    .ok_or("Нет id пользователя — войдите в аккаунт")?;
  let main_dc_id = session
    .get("mainDcId")
    .and_then(Value::as_i64)
    .filter(|dc| (1..=5).contains(dc))
    .ok_or("Некорректный основной DC")? as i32;
  let mut keys: Vec<(i32, [u8; 256])> = session
    .get("keys")
    .and_then(Value::as_object)
    .map(|obj| {
      obj
        .iter()
        .filter_map(|(dc, key)| Some((dc.parse().ok()?, crate::tdata::parse_auth_key(key.as_str()?)?)))
        .collect()
    })
    .unwrap_or_default();
  keys.sort_by_key(|(dc, _)| *dc);
  if !keys.iter().any(|(dc, _)| *dc == main_dc_id) {
    return Err("Нет ключа авторизации для основного DC".into());
  }

  let picked = app.dialog().file().set_title("Куда сохранить tdata").blocking_pick_folder();
  let Some(folder) = picked.and_then(|f| f.into_path().ok()) else { return Ok(Value::Null) };
  let target = folder.join("tdata");
  if fs::read_dir(&target).is_ok_and(|mut entries| entries.next().is_some()) {
    return Err(format!("Папка {} уже существует и не пуста", target.display()));
  }

  crate::tdata::write(&target, &crate::tdata::Session { user_id, main_dc_id, keys })?;
  Ok(json!(target.to_string_lossy()))
}
