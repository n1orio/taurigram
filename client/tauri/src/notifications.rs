// Кастомные всплывающие уведомления: прозрачное окно поверх всех в правом нижнем углу экрана.

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
