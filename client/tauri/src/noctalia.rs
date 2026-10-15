// Синхронизация цветов с Noctalia: её шаблон Telegram пишет палитру в
// `$XDG_CONFIG_HOME/telegram-desktop/themes/noctalia.tdesktop-theme`.
// Файл опрашивается по mtime, изменения рассылаются окнам событием.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tauri::{AppHandle, Emitter};

pub const CHANGED_EVENT: &str = "ethernet:noctalia-theme-changed";
const POLL_INTERVAL: Duration = Duration::from_secs(1);

fn theme_path() -> Option<PathBuf> {
  let config = std::env::var_os("XDG_CONFIG_HOME")
    .filter(|v| !v.is_empty())
    .map(PathBuf::from)
    .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
  Some(config.join("telegram-desktop").join("themes").join("noctalia.tdesktop-theme"))
}

fn modified_at() -> Option<SystemTime> {
  fs::metadata(theme_path()?).and_then(|m| m.modified()).ok()
}

pub fn read() -> Option<String> {
  fs::read_to_string(theme_path()?).ok()
}

pub fn start_watcher(app: AppHandle) {
  std::thread::spawn(move || {
    let mut last = modified_at();
    loop {
      std::thread::sleep(POLL_INTERVAL);
      let current = modified_at();
      if current != last {
        last = current;
        let _ = app.emit(CHANGED_EVENT, ());
      }
    }
  });
}
