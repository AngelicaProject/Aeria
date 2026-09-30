//! Where Aeria keeps its local data and caches.
//!
//! Data lives in a folder named after the application in the platform data
//! directory (`%APPDATA%\Aeria` on Windows), and disposable caches in the same
//! folder under the platform cache directory (`%LOCALAPPDATA%\Aeria`). The
//! bundle identifier, which also names the `WebView` data folder and the
//! installer registration, is unchanged.
//!
//! Earlier versions used a data folder named after the bundle identifier. It
//! is moved once at startup by [`migrate_legacy_directories`]; until a move
//! succeeds, the legacy data folder remains in use so nothing is lost.

use std::fs;
use std::path::{Path, PathBuf};

use tauri::Manager;

#[cfg(windows)]
const APP_FOLDER: &str = "Aeria";
#[cfg(not(windows))]
const APP_FOLDER: &str = "aeria";

/// Resolves Aeria's data and cache folders.
pub(crate) trait AeriaPaths {
    /// The folder for settings and registries.
    fn aeria_data_dir(&self) -> tauri::Result<PathBuf>;
    /// The folder for disposable caches.
    fn aeria_cache_dir(&self) -> tauri::Result<PathBuf>;
}

impl<R: tauri::Runtime> AeriaPaths for tauri::AppHandle<R> {
    fn aeria_data_dir(&self) -> tauri::Result<PathBuf> {
        let current = self.path().data_dir()?.join(APP_FOLDER);
        let legacy = self.path().app_data_dir()?;
        Ok(choose_data_dir(current, &legacy))
    }

    fn aeria_cache_dir(&self) -> tauri::Result<PathBuf> {
        Ok(self.path().cache_dir()?.join(APP_FOLDER))
    }
}

/// The bundle identifier, which named the data folder of earlier versions.
const LEGACY_FOLDER: &str = "org.angelicaproject.aeria";

/// The data folder without a running application, for the `aeria` command:
/// the same folder [`AeriaPaths::aeria_data_dir`] resolves.
pub(crate) fn standalone_data_dir() -> Option<PathBuf> {
    let base = dirs::data_dir()?;
    Some(choose_data_dir(
        base.join(APP_FOLDER),
        &base.join(LEGACY_FOLDER),
    ))
}

/// Uses the legacy folder only while it still holds data that was not moved.
fn choose_data_dir(current: PathBuf, legacy: &Path) -> PathBuf {
    if !current.exists() && legacy.is_dir() && legacy != current {
        legacy.to_owned()
    } else {
        current
    }
}

/// Moves data from the folder named after the bundle identifier. Failures
/// are reported and leave the legacy folder in place.
pub(crate) fn migrate_legacy_directories<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let paths = app.path();
    if let (Ok(data), Ok(legacy)) = (paths.data_dir(), paths.app_data_dir())
        && let Err(error) = migrate_data(&legacy, &data.join(APP_FOLDER))
    {
        eprintln!(
            "could not move Aeria data from {}: {error}",
            legacy.display()
        );
    }
}

pub(crate) fn migrate_data(legacy: &Path, current: &Path) -> std::io::Result<()> {
    if legacy == current {
        return Ok(());
    }
    move_if_absent(legacy, current)
}

/// Renames `from` to `to` when `from` exists and `to` does not.
fn move_if_absent(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.exists() || to.exists() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_data_is_used_only_until_it_is_moved() {
        let directory = tempfile::tempdir().expect("temp dir");
        // A Russian Windows profile puts both folders under a Cyrillic path.
        let profile = directory.path().join("Анна Иванова").join("AppData");
        let legacy = profile.join("org.example.aeria");
        let current = profile.join("Aeria");
        assert_eq!(choose_data_dir(current.clone(), &legacy), current);
        fs::create_dir_all(&legacy).expect("legacy");
        assert_eq!(choose_data_dir(current.clone(), &legacy), legacy);

        fs::write(legacy.join("settings.json"), b"x").expect("settings");
        migrate_data(&legacy, &current).expect("migrate");
        assert!(!legacy.exists());
        assert!(current.join("settings.json").is_file());
        assert_eq!(choose_data_dir(current.clone(), &legacy), current);

        // A second run, or one where both folders exist, changes nothing.
        fs::create_dir_all(&legacy).expect("legacy again");
        migrate_data(&legacy, &current).expect("idempotent");
        assert!(legacy.exists());
        assert_eq!(choose_data_dir(current.clone(), &legacy), current);
    }
}
