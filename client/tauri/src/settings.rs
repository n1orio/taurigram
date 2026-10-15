// Единый менеджер taurigram-settings.json.
// Файл перечитывается при каждом обращении (источник истины — диск), запись сериализуется мьютексом.

use std::fs;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

static LOCK: Mutex<()> = Mutex::new(());

type Obj = Map<String, Value>;

pub fn read() -> Obj {
  let path = &crate::paths::get().settings_file;
  fs::read_to_string(path)
    .ok()
    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    .and_then(|v| match v {
      Value::Object(m) => Some(m),
      _ => None,
    })
    .unwrap_or_else(|| {
      let mut m = Map::new();
      m.insert("enabledPlugins".into(), json!([]));
      m.insert("theme".into(), Value::Null);
      m
    })
}

fn write(s: &Obj) {
  let path = &crate::paths::get().settings_file;
  match serde_json::to_string_pretty(s) {
    Ok(content) => {
      if let Err(err) = fs::write(path, content) {
        log::error!("[settings] write error: {err}");
      }
    }
    Err(err) => log::error!("[settings] serialize error: {err}"),
  }
}

fn update<T>(f: impl FnOnce(&mut Obj) -> T) -> T {
  let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
  let mut s = read();
  let result = f(&mut s);
  write(&s);
  result
}

// --- JS-подобные помощники ---

/// Непустая строка (аналог truthy-проверки в JS)
fn str_of(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn truthy(v: Option<&Value>) -> bool {
  match v {
    None | Some(Value::Null) => false,
    Some(Value::Bool(b)) => *b,
    Some(Value::String(s)) => !s.is_empty(),
    Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
    Some(_) => true,
  }
}

fn obj_mut<'a>(m: &'a mut Obj, key: &str) -> &'a mut Obj {
  if !m.get(key).is_some_and(Value::is_object) {
    m.insert(key.into(), Value::Object(Map::new()));
  }
  m.get_mut(key).and_then(Value::as_object_mut).unwrap()
}

/// `{ ...a, ...b }`
fn merge(a: Option<&Value>, b: &Value) -> Value {
  let mut out = a.and_then(Value::as_object).cloned().unwrap_or_default();
  if let Some(b) = b.as_object() {
    for (k, v) in b {
      out.insert(k.clone(), v.clone());
    }
  }
  Value::Object(out)
}

fn is_video(file: &str) -> bool {
  let lower = file.to_lowercase();
  lower.ends_with(".mp4") || lower.ends_with(".webm")
}

fn is_non_empty_obj(v: Option<&Value>) -> bool {
  v.and_then(Value::as_object).is_some_and(|o| !o.is_empty())
}

fn theme_key(s: &Obj, theme: Option<&str>) -> String {
  theme
    .filter(|t| !t.is_empty())
    .map(str::to_string)
    .or_else(|| str_of(s.get("theme")))
    .unwrap_or_else(|| "default".into())
}

// --- Темы ---

pub fn get_theme() -> Option<String> {
  str_of(read().get("theme"))
}

pub fn set_theme(theme: Option<String>) {
  update(|s| {
    let old_theme = str_of(s.get("theme")).unwrap_or_else(|| "default".into());
    let current_mod = s.get("mod").cloned();
    let theme_mods = obj_mut(s, "themeMods");
    if truthy(current_mod.as_ref()) {
      let merged = merge(theme_mods.get(&old_theme), current_mod.as_ref().unwrap());
      theme_mods.insert(old_theme, merged);
    }
    let new_theme = theme.clone().unwrap_or_else(|| "default".into());
    let new_mod = theme_mods.get(&new_theme).cloned().map(|m| merge(None, &m)).unwrap_or_else(|| json!({}));
    s.insert("theme".into(), theme.map(Value::String).unwrap_or(Value::Null));
    s.insert("mod".into(), new_mod);
  });
}

/// Параметры мода (скругления, блюр, анимации) — применяются лоадером
pub fn get_mod(theme: Option<&str>) -> Value {
  let s = read();
  let t = theme_key(&s, theme);
  if let Some(m) = s.get("themeMods").and_then(|tm| tm.get(&t)) {
    if is_non_empty_obj(Some(m)) {
      return m.clone();
    }
  }
  if is_non_empty_obj(s.get("mod")) {
    return s["mod"].clone();
  }
  json!({
    "colors": {
      "--color-background": "#16171a",
      "--color-background-secondary": "#212328",
      "--color-background-secondary-accent": "#292c33",
      "--color-background-sidebar": "#1a1b1f",
      "--color-background-selected": "#2a2e37",
      "--color-borders": "#2a2d34",
      "--color-dividers": "#24272e",
      "--color-text": "#f3f4f6",
      "--color-links": "#58a6ff",
      "--color-text-secondary": "#9da7b7",
      "--color-primary": "#3b82f6",
      "--color-icon-buttons": "#9da7b7",
      "--color-text-meta-colored": "#58a6ff",
      "--color-background-own": "#1e3a5f",
      "--color-chat-active": "#2b3d58"
    },
    "radii": { "ui": 16, "messages": 16, "buttons": 12, "avatars": 50 }
  })
}

pub fn set_mod(new_mod: &Value, theme: Option<&str>) -> Value {
  update(|s| {
    let t = theme_key(s, theme);
    let is_current = theme.filter(|x| !x.is_empty()).is_none()
      || theme == Some(str_of(s.get("theme")).as_deref().unwrap_or("default"));
    let theme_mods = obj_mut(s, "themeMods");
    let merged = merge(theme_mods.get(&t), new_mod);
    theme_mods.insert(t, merged.clone());
    if is_current {
      s.insert("mod".into(), merged.clone());
    }
    merged
  })
}

// --- Обои, привязанные к теме ---

pub fn get_theme_wallpaper(theme: Option<&str>) -> Option<Value> {
  let s = read();
  let t = theme_key(&s, theme);
  if let Some(tw) = s.get("themeWallpapers").and_then(|tw| tw.get(&t)) {
    if truthy(tw.get("file")) || truthy(tw.get("slug")) {
      return Some(tw.clone());
    }
  }
  let m = s.get("mod");
  let mod_file = str_of(m.and_then(|m| m.get("wallpaperFile")));
  let slug = str_of(s.get("wallpaper")).or_else(|| mod_file.clone())?;
  let file = mod_file.unwrap_or_else(|| if slug.contains('.') { slug.clone() } else { format!("{slug}.png") });
  let kind = str_of(m.and_then(|m| m.get("wallpaperKind")))
    .unwrap_or_else(|| if is_video(&file) { "video".into() } else { "image".into() });
  let original_path = str_of(m.and_then(|m| m.get("wallpaperOriginalPath"))).unwrap_or_else(|| file.clone());
  Some(json!({ "slug": slug, "file": file, "kind": kind, "originalPath": original_path }))
}

pub fn set_theme_wallpaper(theme: Option<&str>, info: Option<&Value>) -> Value {
  update(|s| {
    let t = theme_key(s, theme);
    match info {
      Some(info) => {
        obj_mut(s, "themeWallpapers").insert(t.clone(), info.clone());
        let wallpaper = info.get("slug").filter(|v| truthy(Some(v))).or_else(|| info.get("file")).cloned();
        s.insert("wallpaper".into(), wallpaper.unwrap_or(Value::Null));
        let m = obj_mut(s, "mod");
        for (dst, src) in [
          ("wallpaperFile", "file"),
          ("wallpaperKind", "kind"),
          ("wallpaperOriginalPath", "originalPath"),
        ] {
          match info.get(src) {
            Some(v) => m.insert(dst.into(), v.clone()),
            None => m.remove(dst),
          };
        }
      }
      None => {
        obj_mut(s, "themeWallpapers").remove(&t);
        s.insert("wallpaper".into(), Value::Null);
        if let Some(m) = s.get_mut("mod").and_then(Value::as_object_mut) {
          m.remove("wallpaperFile");
          m.remove("wallpaperKind");
          m.remove("wallpaperOriginalPath");
        }
      }
    }
    s.get("themeWallpapers").and_then(|tw| tw.get(&t)).cloned().unwrap_or(Value::Null)
  })
}

// --- Плагины ---

fn enabled_plugins(s: &Obj) -> Vec<String> {
  s.get("enabledPlugins")
    .and_then(Value::as_array)
    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
    .unwrap_or_default()
}

pub fn is_plugin_enabled(id: &str) -> bool {
  enabled_plugins(&read()).iter().any(|p| p == id)
}

pub fn toggle_plugin(id: &str) -> bool {
  update(|s| {
    let mut arr = enabled_plugins(s);
    let enabled = if let Some(i) = arr.iter().position(|p| p == id) {
      arr.remove(i);
      false
    } else {
      arr.push(id.to_string());
      true
    };
    s.insert("enabledPlugins".into(), json!(arr));
    enabled
  })
}

pub fn remove_plugin(id: &str) {
  update(|s| {
    let arr: Vec<String> = enabled_plugins(s).into_iter().filter(|p| p != id).collect();
    s.insert("enabledPlugins".into(), json!(arr));
  });
}

// --- Встроенный прокси (SOCKS5, HTTP) ---

fn default_proxy_settings() -> Value {
  json!({ "enabled": false, "activeProxyId": null, "proxies": [] })
}

fn proxy_settings_mut(s: &mut Obj) -> &mut Obj {
  if !s.get("proxySettings").is_some_and(Value::is_object) {
    s.insert("proxySettings".into(), default_proxy_settings());
  }
  s.get_mut("proxySettings").and_then(Value::as_object_mut).unwrap()
}

pub fn get_proxy_settings() -> Value {
  read().get("proxySettings").filter(|v| v.is_object()).cloned().unwrap_or_else(default_proxy_settings)
}

/// Активный прокси, если встроенный прокси включён
pub fn active_proxy() -> Option<Value> {
  let ps = get_proxy_settings();
  if !truthy(ps.get("enabled")) {
    return None;
  }
  let active_id = ps.get("activeProxyId").filter(|v| truthy(Some(v)))?;
  ps.get("proxies")?.as_array()?.iter().find(|p| p.get("id") == Some(active_id)).cloned()
}

pub fn save_proxy(config: Value) -> Value {
  update(|s| {
    let ps = proxy_settings_mut(s);
    let mut list = ps.get("proxies").and_then(Value::as_array).cloned().unwrap_or_default();
    let id = config.get("id").cloned();
    if let Some(i) = list.iter().position(|p| p.get("id") == id.as_ref()) {
      list[i] = config;
    } else {
      list.push(config);
    }
    ps.insert("proxies".into(), Value::Array(list));
    if !truthy(ps.get("activeProxyId")) {
      ps.insert("activeProxyId".into(), id.unwrap_or(Value::Null));
    }
    Value::Object(ps.clone())
  })
}

pub fn delete_proxy(id: &Value) -> Value {
  update(|s| {
    if !s.get("proxySettings").is_some_and(Value::is_object) {
      return default_proxy_settings();
    }
    let ps = proxy_settings_mut(s);
    let list: Vec<Value> = ps
      .get("proxies")
      .and_then(Value::as_array)
      .map(|a| a.iter().filter(|p| p.get("id") != Some(id)).cloned().collect())
      .unwrap_or_default();
    if ps.get("activeProxyId") == Some(id) || list.is_empty() {
      ps.insert("activeProxyId".into(), Value::Null);
      ps.insert("enabled".into(), json!(false));
    }
    ps.insert("proxies".into(), Value::Array(list));
    Value::Object(ps.clone())
  })
}

pub fn set_active_proxy(id: Value) -> Value {
  update(|s| {
    let ps = proxy_settings_mut(s);
    ps.insert("activeProxyId".into(), id);
    // Автоматически включаем при выборе прокси
    ps.insert("enabled".into(), json!(true));
    Value::Object(ps.clone())
  })
}

pub fn toggle_proxy(enabled: Option<bool>) -> Value {
  update(|s| {
    let ps = proxy_settings_mut(s);
    let next = enabled.unwrap_or_else(|| !truthy(ps.get("enabled")));
    ps.insert("enabled".into(), json!(next));
    Value::Object(ps.clone())
  })
}

// --- Флаги ---

fn get_flag(key: &str) -> bool {
  truthy(read().get(key))
}

fn set_flag(key: &str, value: bool) -> bool {
  update(|s| {
    s.insert(key.into(), json!(value));
    value
  })
}

pub fn get_notifications_disabled() -> bool {
  get_flag("disableAllNotifications")
}

pub fn set_notifications_disabled(disabled: bool) -> bool {
  set_flag("disableAllNotifications", disabled)
}

pub fn get_close_to_tray_disabled() -> bool {
  get_flag("disableCloseToTray")
}

pub fn set_close_to_tray_disabled(disabled: bool) -> bool {
  set_flag("disableCloseToTray", disabled)
}

// --- Безопасный режим (Safe Mode) для плагинов ---

pub fn get_safe_mode() -> bool {
  get_flag("safeMode")
}

pub fn set_safe_mode(enabled: bool) -> bool {
  set_flag("safeMode", enabled)
}

pub fn get_last_crashed_plugin() -> Value {
  read().get("lastCrashedPlugin").cloned().unwrap_or(Value::Null)
}

pub fn clear_last_crashed_plugin() {
  update(|s| {
    s.remove("lastCrashedPlugin");
  });
}

pub fn report_plugin_crash(plugin_id: &str, error: &str) -> Value {
  update(|s| {
    // Отключаем упавший плагин и включаем Safe Mode для защиты от повторных крашей
    let arr: Vec<String> = enabled_plugins(s).into_iter().filter(|p| p != plugin_id).collect();
    s.insert("enabledPlugins".into(), json!(arr));
    s.insert("safeMode".into(), json!(true));
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let crashed = json!({
      "id": plugin_id,
      "error": if error.is_empty() { "Unknown error" } else { error },
      "time": time,
    });
    s.insert("lastCrashedPlugin".into(), crashed.clone());
    crashed
  })
}
