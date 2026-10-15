// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Видео на NVIDIA: аппаратный декодер (NVDEC) даёт артефакты, а программный с GL-синком WebKit —
// зелёный кадр. Надёжно работает программное декодирование без GL-синка
#[cfg(target_os = "linux")]
const NVDEC_DISABLED_RANKS: &str = "nvh264dec:NONE,nvh265dec:NONE,nvav1dec:NONE,nvvp9dec:NONE,nvvp8dec:NONE,\
nvmpeg2videodec:NONE,nvmpeg4videodec:NONE,nvh264sldec:NONE,nvh265sldec:NONE,nvav1sldec:NONE,nvvp9sldec:NONE";

fn main() {
  #[cfg(target_os = "linux")]
  if std::path::Path::new("/proc/driver/nvidia/version").exists() {
    // WebKitGTK на Wayland с драйвером NVIDIA падает с Error 71 из-за явной синхронизации.
    // Отключаем только её: аппаратный DMABUF-рендерер остаётся (без него интерфейс заметно медленнее).
    // Если не помогает, рендерер можно выключить вручную: WEBKIT_DISABLE_DMABUF_RENDERER=1
    if std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
      unsafe { std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1") };
    }
    // Свои значения можно задать через GST_PLUGIN_FEATURE_RANK и WEBKIT_GST_DISABLE_GL_SINK
    if std::env::var_os("GST_PLUGIN_FEATURE_RANK").is_none() {
      unsafe { std::env::set_var("GST_PLUGIN_FEATURE_RANK", NVDEC_DISABLED_RANKS) };
    }
    if std::env::var_os("WEBKIT_GST_DISABLE_GL_SINK").is_none() {
      unsafe { std::env::set_var("WEBKIT_GST_DISABLE_GL_SINK", "1") };
    }
  }

  app_lib::run();
}
