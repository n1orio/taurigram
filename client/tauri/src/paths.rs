// Расположение пользовательских данных Taurigram: themes/, plugins/, wallpapers/ и taurigram-settings.json.
//
// Порядок выбора корня:
//   1. `TAURIGRAM_ROOT` из окружения;
//   2. debug-сборка — корень репозитория (как `npm run app` у Electron-версии);
//   3. папка AppImage-файла;
//   4. папка исполняемого файла, если в неё можно писать (portable / tar.gz);
//   5. каталог данных приложения (установленные deb/NSIS-версии).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::Manager;

pub struct Paths {
  pub root: PathBuf,
  /// Собранный веб-клиент (`npm run app:build`)
  pub dist: PathBuf,
  pub themes: PathBuf,
  pub plugins: PathBuf,
  pub wallpapers: PathBuf,
  pub settings_file: PathBuf,
}

static PATHS: OnceLock<Paths> = OnceLock::new();

const SETTINGS_FILE_NAME: &str = "taurigram-settings.json";
// Настройки Ethernet, из которых Taurigram вырос, подхватываются при первом запуске
const LEGACY_SETTINGS_FILE_NAME: &str = "ethernet-settings.json";

pub fn get() -> &'static Paths {
  PATHS.get().expect("paths::init must be called first")
}

pub fn init(app: &tauri::AppHandle) {
  let root = resolve_root(app);
  let paths = Paths {
    dist: resolve_dist(app),
    themes: root.join("themes"),
    plugins: root.join("plugins"),
    wallpapers: root.join("wallpapers"),
    settings_file: root.join(SETTINGS_FILE_NAME),
    root,
  };
  log::info!("[paths] Taurigram root: {}, dist: {}", paths.root.display(), paths.dist.display());
  let _ = PATHS.set(paths);
  ensure_dirs();
  seed_default_assets(app);
}

pub fn ensure_dirs() {
  let p = get();
  for dir in [&p.themes, &p.plugins, &p.wallpapers] {
    let _ = fs::create_dir_all(dir);
  }
}

fn resolve_root(app: &tauri::AppHandle) -> PathBuf {
  if let Some(root) = std::env::var_os("TAURIGRAM_ROOT") {
    return PathBuf::from(root);
  }

  if cfg!(debug_assertions) {
    // client/tauri -> корень репозитория
    return Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
  }

  if let Some(appimage) = std::env::var_os("APPIMAGE") {
    if let Some(dir) = Path::new(&appimage).parent() {
      if is_writable(dir) {
        return dir.to_path_buf();
      }
    }
  }

  if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
    if is_writable(&dir) && !is_system_dir(&dir) {
      return dir;
    }
  }

  app
    .path()
    .app_data_dir()
    .unwrap_or_else(|_| std::env::temp_dir().join("taurigram"))
}

fn resolve_dist(app: &tauri::AppHandle) -> PathBuf {
  if let Some(dist) = std::env::var_os("TAURIGRAM_DIST") {
    return PathBuf::from(dist);
  }
  let dev_dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("dist");
  if cfg!(debug_assertions) {
    return dev_dist;
  }
  app
    .path()
    .resource_dir()
    .map(|res| res.join("dist"))
    .ok()
    .filter(|dir| dir.is_dir())
    .unwrap_or(dev_dist)
}

fn is_writable(dir: &Path) -> bool {
  let probe = dir.join(format!(".taurigram-write-test-{}", std::process::id()));
  let ok = fs::write(&probe, b"test").is_ok();
  let _ = fs::remove_file(&probe);
  ok
}

fn is_system_dir(dir: &Path) -> bool {
  cfg!(unix) && (dir.starts_with("/usr") || dir.starts_with("/bin") || dir.starts_with("/opt"))
}

fn bundle_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
  let mut candidates = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("assets-bundle")];
  if let Ok(res) = app.path().resource_dir() {
    candidates.insert(0, res.join("assets-bundle"));
  }
  candidates.into_iter().find(|dir| dir.is_dir())
}

// Копирует стандартные темы/обои/плагины/настройки из assets-bundle, не перезаписывая существующие
fn seed_default_assets(app: &tauri::AppHandle) {
  let p = get();

  let legacy_settings = p.root.join(LEGACY_SETTINGS_FILE_NAME);
  if !p.settings_file.exists() && legacy_settings.exists() {
    let _ = fs::copy(&legacy_settings, &p.settings_file);
  }

  let Some(bundle) = bundle_dir(app) else { return };

  let bundle_settings = bundle.join(SETTINGS_FILE_NAME);
  if !p.settings_file.exists() && bundle_settings.exists() {
    let _ = fs::copy(&bundle_settings, &p.settings_file);
  }

  for (sub, dest_dir) in [("themes", &p.themes), ("wallpapers", &p.wallpapers)] {
    let Ok(entries) = fs::read_dir(bundle.join(sub)) else { continue };
    for entry in entries.flatten() {
      let dest = dest_dir.join(entry.file_name());
      if entry.path().is_file() && !dest.exists() {
        let _ = fs::copy(entry.path(), dest);
      }
    }
  }

  if let Ok(entries) = fs::read_dir(bundle.join("plugins")) {
    for entry in entries.flatten() {
      let dest = p.plugins.join(entry.file_name());
      if entry.path().is_dir() && !dest.exists() {
        let _ = copy_dir(&entry.path(), &dest);
      }
    }
  }
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
  fs::create_dir_all(dest)?;
  for entry in fs::read_dir(src)?.flatten() {
    let target = dest.join(entry.file_name());
    if entry.path().is_dir() {
      copy_dir(&entry.path(), &target)?;
    } else {
      fs::copy(entry.path(), target)?;
    }
  }
  Ok(())
}

/// Проверяет, что `target` лежит строго внутри `base` (защита от path traversal)
pub fn is_inside(base: &Path, target: &Path) -> bool {
  let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
  let parent = target.parent().map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
  match (parent, target.file_name()) {
    (Some(parent), Some(name)) => parent.join(name).starts_with(&base) && parent.join(name) != base,
    _ => false,
  }
}
