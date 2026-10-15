// Всплывающие уведомления: прозрачное окно поверх всех в правом нижнем углу экрана.
// На Wayland приложение не может задать позицию своего окна, поэтому там
// используются системные уведомления (D-Bus) с переходом в чат по клику.

use std::sync::Mutex;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::settings;

pub const LABEL: &str = "notifications";

const WIDTH: f64 = 380.0;
const MAX_HEIGHT: f64 = 360.0;
const SCREEN_MARGIN: f64 = 16.0;
const CARD_PADDING: f64 = 20.0;

static NOTIF_BRIDGE_JS: &str = include_str!("../web/notif-bridge.js");

#[derive(Default)]
struct NotifState {
  is_ready: bool,
  pending: Vec<Value>,
}

static STATE: Mutex<NotifState> = Mutex::new(NotifState { is_ready: false, pending: Vec::new() });

pub fn show(app: &AppHandle, payload: Value) {
  if settings::get_notifications_disabled() {
    return;
  }
  #[cfg(target_os = "linux")]
  if is_wayland() {
    native::show(app, payload);
    return;
  }
  if ensure_window(app).is_none() {
    return;
  }
  let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
  if state.is_ready {
    let _ = app.emit_to(LABEL, "notif:add", payload);
  } else {
    state.pending.push(payload);
  }
}

/// Окно уведомлений загрузилось — отдаём накопленные уведомления
pub fn mark_ready(app: &AppHandle) {
  let pending = {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.is_ready = true;
    std::mem::take(&mut state.pending)
  };
  for payload in pending {
    let _ = app.emit_to(LABEL, "notif:add", payload);
  }
}

pub fn set_height(app: &AppHandle, height: f64) {
  let Some(win) = app.get_webview_window(LABEL) else { return };
  if height <= 0.0 {
    let _ = win.hide();
    return;
  }
  let win_height = (height + CARD_PADDING).min(MAX_HEIGHT);
  place(&win, win_height);
  if !win.is_visible().unwrap_or(false) {
    let _ = win.show();
  }
}

pub fn handle_click(app: &AppHandle, data: Value) {
  if let Some(main) = crate::show_main_window(app) {
    let _ = main.emit_to(crate::MAIN_WINDOW_LABEL, "hermes:notification-clicked", data);
  }
}

fn ensure_window(app: &AppHandle) -> Option<WebviewWindow> {
  if let Some(win) = app.get_webview_window(LABEL) {
    return Some(win);
  }
  {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.is_ready = false;
  }
  let url = crate::protocol::base_url().join("ethernet/notifications.html").ok()?;
  let builder = crate::app_webview_builder(app, LABEL, url)
    .initialization_script(NOTIF_BRIDGE_JS)
    .title("Notifications")
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .resizable(false)
    .focused(false)
    .focusable(false)
    .visible(false)
    .inner_size(WIDTH, MAX_HEIGHT);

  match builder.build() {
    Ok(win) => {
      place(&win, MAX_HEIGHT);
      Some(win)
    }
    Err(err) => {
      log::error!("[notifications] failed to create window: {err}");
      None
    }
  }
}

fn place(win: &WebviewWindow, logical_height: f64) {
  let monitor = win.primary_monitor().ok().flatten().or_else(|| win.current_monitor().ok().flatten());
  let Some(monitor) = monitor else { return };
  let scale = monitor.scale_factor();
  let area = monitor.work_area();
  let width = (WIDTH * scale).round();
  let height = (logical_height * scale).round();
  let margin = (SCREEN_MARGIN * scale).round();
  let x = area.position.x as f64 + area.size.width as f64 - width - margin;
  let y = area.position.y as f64 + area.size.height as f64 - height - margin;
  let _ = win.set_size(PhysicalSize::new(width as u32, height as u32));
  let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
}

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
  let backend_is_x11 = std::env::var("GDK_BACKEND").is_ok_and(|b| b.starts_with("x11"));
  let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
    || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");
  has_wayland && !backend_is_x11
}

#[cfg(target_os = "linux")]
mod native {
  use std::path::PathBuf;

  use base64::Engine;
  use notify_rust::{Hint, Notification};
  use serde_json::{Value, json};
  use tauri::{AppHandle, Manager};

  const DEFAULT_ACTION: &str = "default";

  pub fn show(app: &AppHandle, payload: Value) {
    let text = |k: &str| payload.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let click_data = json!({ "chatId": payload.get("chatId"), "messageId": payload.get("messageId") });
    let is_silent = payload.get("isSilent").and_then(Value::as_bool).unwrap_or(false);
    let avatar = save_avatar(app, &text("icon"));
    let (title, body) = (text("title"), text("body"));
    let app = app.clone();

    // Ожидание клика блокирует поток до закрытия уведомления
    std::thread::spawn(move || {
      let mut notification = Notification::new();
      notification
        .appname("Taurigram")
        .summary(&title)
        .body(&body)
        .icon("taurigram")
        .action(DEFAULT_ACTION, "Открыть");
      if let Some(path) = avatar.as_ref().and_then(|p| p.to_str()) {
        notification.image_path(path);
      }
      if is_silent {
        notification.hint(Hint::SuppressSound(true));
      }
      match notification.show() {
        Ok(handle) => handle.wait_for_action(|action| {
          if action == DEFAULT_ACTION {
            super::handle_click(&app, click_data);
          }
        }),
        Err(err) => log::error!("[notifications] D-Bus notification failed: {err}"),
      }
      if let Some(path) = avatar {
        let _ = std::fs::remove_file(path);
      }
    });
  }

  /// Аватар приходит как `data:image/...;base64,...` — сохраняем во временный файл для `image-path`
  fn save_avatar(app: &AppHandle, icon: &str) -> Option<PathBuf> {
    let (meta, data) = icon.strip_prefix("data:")?.split_once(',')?;
    let ext = match meta.split(';').next()? {
      "image/png" => "png",
      "image/jpeg" => "jpg",
      "image/webp" => "webp",
      _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
    let dir = app.path().app_cache_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("notification-{}.{ext}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
  }
}
