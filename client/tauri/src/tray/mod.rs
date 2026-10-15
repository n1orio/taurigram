use std::sync::{LazyLock, Mutex};

use tauri::{
  AppHandle, Manager,
  image::Image,
  menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
  tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};

#[cfg(not(target_os = "macos"))]
mod badge;

use crate::settings;

#[cfg(target_os = "macos")]
pub(crate) static TRAY_ICON_BYTES: &[u8] = include_bytes!("../../icons/tray-macos.png");

#[cfg(not(target_os = "macos"))]
pub(crate) static TRAY_ICON_BYTES: &[u8] = include_bytes!("../../icons/32x32.png");

pub(crate) static TRAY_BASE_ICON: LazyLock<Image<'static>> =
  LazyLock::new(|| Image::from_bytes(TRAY_ICON_BYTES).expect("Failed to load base tray icon"));

const TRAY_TOOLTIP: &str = "Ethernet";

pub const MENU_ITEM_OPEN_ID: &str = "open";
pub const MENU_ITEM_NOTIFICATIONS_ID: &str = "notifications";
pub const MENU_ITEM_QUIT_ID: &str = "quit";

pub(super) static TRAY_HANDLE: LazyLock<Mutex<Option<TrayIcon>>> = LazyLock::new(|| Mutex::new(None));

static NOTIFICATIONS_ITEM: LazyLock<Mutex<Option<CheckMenuItem<tauri::Wry>>>> = LazyLock::new(|| Mutex::new(None));

pub fn init(app: &AppHandle) -> Result<(), tauri::Error> {
  let open_i = MenuItem::with_id(app, MENU_ITEM_OPEN_ID, "Открыть Ethernet", true, None::<&str>)?;
  let notifications_i = CheckMenuItem::with_id(
    app,
    MENU_ITEM_NOTIFICATIONS_ID,
    "Уведомления",
    true,
    !settings::get_notifications_disabled(),
    None::<&str>,
  )?;
  let quit_i = MenuItem::with_id(app, MENU_ITEM_QUIT_ID, "Выйти", true, None::<&str>)?;
  let separator = PredefinedMenuItem::separator(app)?;
  let menu = Menu::with_items(app, &[&open_i, &notifications_i, &separator, &quit_i])?;

  let tray_builder = TrayIconBuilder::new()
    .icon(TRAY_BASE_ICON.clone())
    .menu(&menu)
    .show_menu_on_left_click(false)
    .tooltip(TRAY_TOOLTIP)
    .on_menu_event(|app, event| match event.id.as_ref() {
      MENU_ITEM_OPEN_ID => {
        crate::show_main_window(app);
      }
      MENU_ITEM_NOTIFICATIONS_ID => toggle_notifications(app),
      MENU_ITEM_QUIT_ID => app.exit(0),
      _ => {}
    })
    .on_tray_icon_event(|tray, event| {
      if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
      | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = event
      {
        crate::show_main_window(tray.app_handle());
      }
    });

  #[cfg(target_os = "macos")]
  let tray_builder = tray_builder.icon_as_template(true);

  let tray_icon = tray_builder.build(app)?;

  if let Ok(mut tray_lock) = TRAY_HANDLE.lock() {
    *tray_lock = Some(tray_icon);
  }
  if let Ok(mut item) = NOTIFICATIONS_ITEM.lock() {
    *item = Some(notifications_i);
  }
  Ok(())
}

fn toggle_notifications(app: &AppHandle) {
  let disabled = settings::set_notifications_disabled(!settings::get_notifications_disabled());
  sync_notifications_item(disabled);
  use tauri::Emitter;
  let _ = app.emit_to(crate::MAIN_WINDOW_LABEL, "hermes:notifications-disabled-changed", disabled);
}

/// Синхронизирует галочку «Уведомления» в меню трея с настройками
pub fn sync_notifications_item(disabled: bool) {
  if let Ok(item) = NOTIFICATIONS_ITEM.lock() {
    if let Some(item) = item.as_ref() {
      let _ = item.set_checked(!disabled);
    }
  }
}

/// Счётчик непрочитанных: подсказка трея, значок на панели задач и иконке трея
pub fn set_unread_count(app: &AppHandle, amount: i32, is_muted: bool) {
  if let Ok(tray) = TRAY_HANDLE.lock() {
    if let Some(tray) = tray.as_ref() {
      let tooltip = if amount > 0 { format!("{TRAY_TOOLTIP} ({amount})") } else { TRAY_TOOLTIP.to_string() };
      let _ = tray.set_tooltip(Some(tooltip));
    }
  }

  let Some(window) = app.get_webview_window(crate::MAIN_WINDOW_LABEL) else { return };

  #[cfg(target_os = "macos")]
  {
    let _ = is_muted;
    let _ = window.set_badge_count(if amount > 0 { Some(amount.into()) } else { None });
  }

  #[cfg(not(target_os = "macos"))]
  {
    #[cfg(target_os = "linux")]
    let _ = window.set_badge_count(if amount > 0 { Some(amount.into()) } else { None });
    badge::set_badge_count_icon(&window, amount, is_muted);
  }
}
