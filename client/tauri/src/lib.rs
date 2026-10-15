use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, OnceLock};

use serde_json::json;
use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, window::Color};
use url::Url;
use uuid::Uuid;

mod commands;
mod deeplink;
mod notifications;
mod paths;
mod protocol;
mod proxy;
mod settings;
mod system_proxy;
mod tdata;
mod tray;

use deeplink::Deeplink;

pub const MAIN_WINDOW_LABEL: &str = "main";
pub const DEFAULT_WINDOW_TITLE: &str = "taurigram";

pub const WINDOW_WIDTH: f64 = 1280.0;
pub const WINDOW_HEIGHT: f64 = 832.0;
pub const WINDOW_MIN_WIDTH: f64 = 480.0;
pub const WINDOW_MIN_HEIGHT: f64 = 400.0;

// Аргументы WebView2 должны совпадать у всех окон (общее окружение WebView2)
const BROWSER_ARGS: &str =
  "--autoplay-policy=no-user-gesture-required --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

const ALLOWED_EXTERNAL_SCHEMES: [&str; 4] = ["http", "https", "tg", "mailto"];

static BRIDGE_JS: &str = include_str!("../web/bridge.js");

pub const WITH_UPDATER: &str = match std::option_env!("WITH_UPDATER") {
  Some(str) => str,
  None => "false",
};

static PROXY_URL: OnceLock<Url> = OnceLock::new();

// Последнее известное состояние «развёрнуто» по окнам (для события смены иконки в топбаре)
static MAXIMIZED: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn run() {
  let app = tauri::Builder::default()
    .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, |_ctx, request, responder| {
      tauri::async_runtime::spawn_blocking(move || responder.respond(protocol::handle(&request)));
    })
    .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
      show_main_window(app);
    }))
    .plugin(tauri_plugin_os::init())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_notification::init())
    .plugin(tauri_plugin_log::Builder::default().build())
    .plugin(
      tauri_plugin_window_state::Builder::default()
        .with_denylist(&[notifications::LABEL])
        .with_state_flags(tauri_plugin_window_state::StateFlags::all() & !tauri_plugin_window_state::StateFlags::VISIBLE)
        .build(),
    )
    .plugin(tauri_plugin_deep_link::init())
    .plugin(tauri_plugin_process::init());

  let app = app.on_window_event(|window, event| match event {
    tauri::WindowEvent::CloseRequested { api, .. } => {
      let label = window.label();
      if label == notifications::LABEL {
        api.prevent_close();
        let _ = window.hide();
      } else if label == MAIN_WINDOW_LABEL {
        if settings::get_close_to_tray_disabled() {
          window.app_handle().exit(0);
        } else {
          api.prevent_close();
          let _ = window.hide();
        }
      }
    }
    tauri::WindowEvent::Resized(_) => {
      let Ok(is_max) = window.is_maximized() else { return };
      let label = window.label().to_string();
      let changed = MAXIMIZED
        .lock()
        .map(|mut m| m.insert(label.clone(), is_max) != Some(is_max))
        .unwrap_or(false);
      if changed {
        let _ = window.emit_to(label.as_str(), "hermes:window-maximized-changed", is_max);
      }
    }
    tauri::WindowEvent::Destroyed => {
      if let Ok(mut m) = MAXIMIZED.lock() {
        m.remove(window.label());
      }
    }
    _ => {}
  });

  let app = app.setup(|app| {
    paths::init(app.handle());

    match proxy::start() {
      Ok(url) => {
        let _ = PROXY_URL.set(url);
      }
      Err(err) => log::error!("[ProxyEngine] failed to start relay: {err}"),
    }

    open_window(app.handle().clone(), MAIN_WINDOW_LABEL, protocol::base_url())
      .map_err(|err| format!("Failed to open main window: {err}"))?;

    let deeplink = Deeplink::init();
    if let Err(err) = deeplink.setup(app.handle()) {
      log::error!("Failed to setup deeplink: {:?}", err);
    }

    if WITH_UPDATER == "true" {
      app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
    }

    tray::init(app.handle())?;

    Ok(())
  });

  let app = app.invoke_handler(tauri::generate_handler![
    mark_title_bar_overlay,
    set_notifications_count,
    set_window_title,
    open_new_window_cmd,
    commands::ethernet_themes_list,
    commands::ethernet_theme_read,
    commands::ethernet_theme_save,
    commands::ethernet_theme_delete,
    commands::ethernet_theme_activate,
    commands::ethernet_plugins_list,
    commands::ethernet_plugin_read,
    commands::ethernet_plugin_save,
    commands::ethernet_plugin_delete,
    commands::ethernet_plugin_toggle,
    commands::ethernet_pick_file,
    commands::ethernet_save_file,
    commands::ethernet_reload,
    commands::ethernet_toggle_devtools,
    commands::ethernet_mod_get,
    commands::ethernet_mod_set,
    commands::ethernet_wallpaper_set_file,
    commands::ethernet_wallpaper_clear,
    commands::ethernet_window_minimize,
    commands::ethernet_window_maximize_toggle,
    commands::ethernet_window_close,
    commands::ethernet_window_is_maximized,
    commands::ethernet_network_geo_info,
    commands::ethernet_proxy_get_state,
    commands::ethernet_proxy_save,
    commands::ethernet_proxy_delete,
    commands::ethernet_proxy_set_active,
    commands::ethernet_proxy_toggle,
    commands::ethernet_proxy_parse_link,
    commands::ethernet_proxy_test_ping,
    commands::ethernet_show_notification,
    commands::ethernet_notifications_disabled_get,
    commands::ethernet_notifications_disabled_set,
    commands::ethernet_close_to_tray_disabled_get,
    commands::ethernet_close_to_tray_disabled_set,
    commands::ethernet_safemode_get,
    commands::ethernet_safemode_set,
    commands::ethernet_last_crashed_plugin_get,
    commands::ethernet_clear_last_crashed_plugin,
    commands::ethernet_report_plugin_crash,
    commands::ethernet_set_unread_count,
    commands::ethernet_export_tdata,
    commands::notif_ready,
    commands::notif_clicked,
    commands::notif_dismissed,
    commands::notif_set_height,
  ]);

  app
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

/// Показывает главное окно (или создаёт его заново) и выводит на передний план
pub(crate) fn show_main_window(app: &AppHandle) -> Option<WebviewWindow> {
  let window = match app.get_webview_window(MAIN_WINDOW_LABEL) {
    Some(window) => window,
    None => open_window(app.clone(), MAIN_WINDOW_LABEL, protocol::base_url()).ok()?,
  };
  let _ = window.unminimize();
  let _ = window.show();
  let _ = window.set_focus();
  Some(window)
}

/// Общие настройки WebView всех окон приложения: протокол app://, прокси-релей, навигация
pub(crate) fn app_webview_builder<'a>(app: &'a AppHandle, label: &str, url: Url) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
  let nav_app = app.clone();
  let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::CustomProtocol(url))
    .additional_browser_args(BROWSER_ARGS)
    .on_navigation(move |url| {
      if protocol::is_app_url(url) {
        return true;
      }
      open_external(&nav_app, url);
      false
    });
  match PROXY_URL.get() {
    Some(proxy_url) => builder.proxy_url(proxy_url.clone()),
    None => builder,
  }
}

fn open_window(app: AppHandle, label: &str, url: Url) -> Result<WebviewWindow, String> {
  let popup_app = app.clone();
  let builder = app_webview_builder(&app, label, url)
    .title(DEFAULT_WINDOW_TITLE)
    .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
    .min_inner_size(WINDOW_MIN_WIDTH, WINDOW_MIN_HEIGHT)
    .decorations(false)
    .background_color(Color(0, 0, 0, 255))
    .resizable(true)
    .disable_drag_drop_handler() // Required for Drag & Drop on Windows
    .initialization_script(format!("window.tauri = {{ version: '{}' }};", env!("CARGO_PKG_VERSION")))
    .initialization_script(BRIDGE_JS)
    .on_new_window(move |url, _features| {
      if protocol::is_app_url(&url) {
        // Окно создаётся вне колбэка WebView, чтобы не блокировать его поток
        let app = popup_app.clone();
        tauri::async_runtime::spawn(async move {
          if let Err(err) = open_window(app, &Uuid::new_v4().to_string(), url) {
            log::error!("Failed to open app window: {err}");
          }
        });
      } else {
        open_external(&popup_app, &url);
      }
      NewWindowResponse::Deny
    })
    .on_download(|window, event| {
      match event {
        #[allow(unused_variables)]
        DownloadEvent::Requested { destination, .. } => {
          // On macOS, Webview does not provide basic download logic
          #[cfg(target_os = "macos")]
          if let Some(filename) = destination.file_name() {
            if let Ok(downloads_dir) = window.app_handle().path().download_dir() {
              *destination = downloads_dir.join(filename);
            }
          }
        }
        DownloadEvent::Finished { url, success, .. } => {
          window
            .emit_to(window.label(), "download-finished", json!({ "url": url.to_string(), "success": success }))
            .unwrap_or_default();
        }
        _ => {}
      };
      true
    });

  builder.build().map_err(|err| err.to_string())
}

/// Открывает внешнюю ссылку в системном браузере/приложении, отбрасывая небезопасные URL
fn open_external(app: &AppHandle, url: &Url) {
  let raw = url.as_str();
  let has_bad_chars = raw.contains('\\') || raw.chars().any(|c| c.is_control() || c.is_whitespace() || "\"'<>".contains(c));
  if !ALLOWED_EXTERNAL_SCHEMES.contains(&url.scheme()) || has_bad_chars {
    log::warn!("[Security] Blocked navigation to unsafe URL: {raw}");
    return;
  }
  #[allow(deprecated)]
  {
    use tauri_plugin_shell::ShellExt;
    if let Err(err) = app.shell().open(raw, None) {
      log::error!("Failed to open external URL: {err}");
    }
  }
}

/// Заголовок окна без хвостов «· Telegram», «[Inactive]» и т.п.
fn clean_title(title: &str) -> String {
  static RE_SUFFIX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)\s*·\s*(telegram(\s+beta)?|taurigram|ethernet)|\s*\[inactive\]").unwrap());
  static RE_PLAIN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)^(telegram(\s+beta)?|taurigram|ethernet)$").unwrap());
  let clean = RE_SUFFIX.replace_all(title, "").trim().to_string();
  if clean.is_empty() || RE_PLAIN.is_match(&clean) { DEFAULT_WINDOW_TITLE.to_string() } else { clean }
}

// --- Команды из апстрима telegram-tt (window.tauri) ---

// Окно без системной рамки — стиль заголовка macOS не используется
#[tauri::command]
#[allow(unused_variables)]
fn mark_title_bar_overlay(is_overlay: bool, is_mobile: Option<bool>) {}

#[tauri::command]
fn set_notifications_count(app: AppHandle, amount: i32, is_muted: bool) {
  tray::set_unread_count(&app, amount, is_muted);
}

#[tauri::command]
fn set_window_title(window: WebviewWindow, title: String) {
  window.set_title(&clean_title(&title)).unwrap_or_default();
}

#[tauri::command]
async fn open_new_window_cmd(app: AppHandle, url: String) -> bool {
  let Ok(url) = protocol::base_url().join(&url) else { return false };
  if !protocol::is_app_url(&url) {
    return false;
  }
  open_window(app, &Uuid::new_v4().to_string(), url).is_ok()
}

#[cfg(test)]
mod tests {
  use super::clean_title;

  #[test]
  fn cleans_titles() {
    assert_eq!(clean_title("Durov · Telegram"), "Durov");
    assert_eq!(clean_title("Durov · Taurigram"), "Durov");
    assert_eq!(clean_title("Telegram Beta"), "taurigram");
    assert_eq!(clean_title("Chat [Inactive]"), "Chat");
    assert_eq!(clean_title(""), "taurigram");
  }
}
