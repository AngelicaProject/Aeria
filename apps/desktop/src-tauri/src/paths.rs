//! Where Aeria keeps its local data and caches.
//!
//! Data lives in a folder named after the application in the platform data
//! directory (`%APPDATA%\Aeria` on Windows), and disposable caches in the same
//! folder under the platform cache directory (`%LOCALAPPDATA%\Aeria`). The
//! bundle identifier, which also names the `WebView` data folder and the
//! installer registration, is unchanged.
//!
//! Earlier versions used folders named after the bundle identifier. They are
//! moved once at startup by [`migrate_legacy_directories`]; until a move
//! succeeds, the legacy data folder remains in use so nothing is lost.

use std::fs;
use std::path::{Path, PathBuf};

use aeria_projects::{ProjectRegistry, REGISTRY_FILE_NAME};
use tauri::Manager;

use crate::commands::SOURCE_PACKAGES_DIRECTORY;

#[cfg(windows)]
const APP_FOLDER: &str = "Aeria";
#[cfg(not(windows))]
const APP_FOLDER: &str = "aeria";

/// Cache folders that move with the legacy cache directory. Other entries
/// there, such as the `WebView` profile, stay under the bundle identifier.
const CACHE_ENTRIES: [&str; 2] = ["hxs", "hsp-verification"];

/// Resolves Aeria's data and cache folders.
pub(crate) trait AeriaPaths {
    /// The folder for settings, registries, and source packages.
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

/// Uses the legacy folder only while it still holds data that was not moved.
fn choose_data_dir(current: PathBuf, legacy: &Path) -> PathBuf {
    if !current.exists() && legacy.is_dir() && legacy != current {
        legacy.to_owned()
    } else {
        current
    }
}

/// Moves data and caches from the folders named after the bundle identifier
/// and points recent projects at the moved source packages. Failures are
/// reported and leave the legacy folders in place.
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
    if let (Ok(cache), Ok(legacy)) = (paths.cache_dir(), paths.app_cache_dir()) {
        let current = cache.join(APP_FOLDER);
        for entry in CACHE_ENTRIES {
            if let Err(error) = move_if_absent(&legacy.join(entry), &current.join(entry)) {
                eprintln!("could not move the Aeria {entry} cache: {error}");
            }
        }
    }
}

pub(crate) fn migrate_data(legacy: &Path, current: &Path) -> std::io::Result<()> {
    if legacy == current {
        return Ok(());
    }
    move_if_absent(legacy, current)?;
    if current.exists() {
        let registry = ProjectRegistry::new(current.join(REGISTRY_FILE_NAME));
        if registry.path().is_file() {
            registry
                .relocate_source_packages(
                    &legacy.join(SOURCE_PACKAGES_DIRECTORY),
                    &current.join(SOURCE_PACKAGES_DIRECTORY),
                )
                .map_err(std::io::Error::other)?;
        }
    }
    Ok(())
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

        fs::create_dir_all(legacy.join(SOURCE_PACKAGES_DIRECTORY)).expect("store");
        fs::write(legacy.join(SOURCE_PACKAGES_DIRECTORY).join("a.hsp"), b"x").expect("package");
        migrate_data(&legacy, &current).expect("migrate");
        assert!(!legacy.exists());
        assert!(
            current
                .join(SOURCE_PACKAGES_DIRECTORY)
                .join("a.hsp")
                .is_file()
        );
        assert_eq!(choose_data_dir(current.clone(), &legacy), current);

        // A second run, or one where both folders exist, changes nothing.
        fs::create_dir_all(&legacy).expect("legacy again");
        migrate_data(&legacy, &current).expect("idempotent");
        assert!(legacy.exists());
        assert_eq!(choose_data_dir(current.clone(), &legacy), current);
    }
}
