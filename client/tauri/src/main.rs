// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
  // WebKitGTK на Wayland с драйвером NVIDIA падает с Error 71 из-за явной синхронизации.
  // Отключаем только её: аппаратный DMABUF-рендерер остаётся (без него интерфейс заметно медленнее).
  // Если не помогает, рендерер можно выключить вручную: WEBKIT_DISABLE_DMABUF_RENDERER=1
  #[cfg(target_os = "linux")]
  if std::path::Path::new("/proc/driver/nvidia/version").exists() && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
    unsafe { std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1") };
  }

  app_lib::run();
}
