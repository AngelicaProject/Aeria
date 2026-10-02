//! The game source of projects: opening the configured installation and the
//! sheet catalog.
//!
//! The catalog lists every sheet with its row and translatable-cell counts.
//! Counting reads every sheet in every evidence language, so the result is
//! kept in the cache directory per source language and game version. The
//! file is disposable: a missing or unreadable one is built again.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aeria_source::{GameSource, SheetSummary, SourceLanguage};
use serde::{Deserialize, Serialize};

use crate::error::CommandError;
use crate::games::resolve_game_path;

type CommandResult<T> = Result<T, CommandError>;

const CATALOG_DIRECTORY: &str = "sheet-catalog";
const CATALOG_FORMAT: u32 = 1;
/// Threads used to count a catalog; more do not read the game faster.
const CATALOG_THREADS: usize = 4;

/// Opens the configured game installation in `language`.
///
/// # Errors
///
/// Returns `gameInstallationRequired` or `gameInstallationInvalid` when no
/// installation is available, `invalidInput` for an unknown language, or
/// `gameRead` when the game cannot be read.
pub(crate) fn open_game(app: &tauri::AppHandle, language: &str) -> CommandResult<Arc<GameSource>> {
    open_game_at(&resolve_game_path(app)?, language)
}

/// Opens the installation at `game_path` in `language`.
///
/// # Errors
///
/// Returns `invalidInput` for an unknown language or `gameRead` when the
/// game cannot be read.
pub(crate) fn open_game_at(game_path: &str, language: &str) -> CommandResult<Arc<GameSource>> {
    let language: SourceLanguage =
        language
            .parse()
            .map_err(|error: aeria_source::SourceLanguageError| {
                CommandError::new("invalidInput", error.to_string())
            })?;
    Ok(Arc::new(GameSource::open(game_path, language)?))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogFile {
    format: u32,
    sheets: Vec<CatalogSheet>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogSheet {
    name: String,
    rows: usize,
    translatable: usize,
    unavailable: bool,
}

fn catalog_path(cache_root: &Path, source: &GameSource) -> PathBuf {
    cache_root.join(CATALOG_DIRECTORY).join(format!(
        "{}-{}.json",
        source.language().code(),
        source.version()
    ))
}

/// Reads a catalog file that lists exactly the game's sheets.
fn read_catalog(path: &Path, source: &GameSource) -> Option<Vec<SheetSummary>> {
    let file: CatalogFile = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let names_match = file.sheets.len() == source.sheet_names().len()
        && file
            .sheets
            .iter()
            .zip(source.sheet_names())
            .all(|(sheet, name)| sheet.name == *name);
    (file.format == CATALOG_FORMAT && names_match).then(|| {
        file.sheets
            .into_iter()
            .map(|sheet| SheetSummary {
                name: sheet.name,
                rows: sheet.rows,
                translatable: sheet.translatable,
                unavailable: sheet.unavailable,
            })
            .collect()
    })
}

fn write_catalog(path: &Path, sheets: &[SheetSummary]) -> std::io::Result<()> {
    let file = CatalogFile {
        format: CATALOG_FORMAT,
        sheets: sheets
            .iter()
            .map(|sheet| CatalogSheet {
                name: sheet.name.clone(),
                rows: sheet.rows,
                translatable: sheet.translatable,
                unavailable: sheet.unavailable,
            })
            .collect(),
    };
    let bytes = serde_json::to_vec(&file).map_err(std::io::Error::other)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let partial = path.with_extension("json.partial");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

/// Supplies the sheet catalog of `source`: from the cache file under
/// `cache_root`, or counted from the game and cached.
///
/// # Errors
///
/// Returns `gameRead` when the game cannot be read.
pub(crate) fn load_catalog_with_cache(cache_root: &Path, source: &GameSource) -> CommandResult<()> {
    if source.catalog().is_some() {
        return Ok(());
    }
    let path = catalog_path(cache_root, source);
    let sheets = if let Some(sheets) = read_catalog(&path, source) {
        sheets
    } else {
        let sheets = source
            .summarize(CATALOG_THREADS, &|| true)?
            .unwrap_or_default();
        // The catalog is a cache; a failed write only costs a recount.
        let _ = write_catalog(&path, &sheets);
        sheets
    };
    source.set_catalog(Arc::new(sheets));
    Ok(())
}

#[cfg(test)]
mod tests {
    use aeria_sqpack::testing::{FakeGame, TextSheet};

    use super::*;

    #[test]
    fn the_catalog_is_counted_once_and_read_back_from_the_cache() {
        let game = tempfile::tempdir().expect("game");
        FakeGame::new("2026.09.15.0000.0000")
            .with_text(
                "Addon",
                &TextSheet::new(2, &[0, 1])
                    .keyed(0)
                    .row(1, &[(0, "K1"), (1, "One")])
                    .row(2, &[(0, "K2"), (1, "Two")]),
            )
            .write(game.path())
            .expect("write");
        let source = GameSource::open(game.path(), SourceLanguage::English).expect("source");
        let cache = tempfile::tempdir().expect("cache");
        load_catalog_with_cache(cache.path(), &source).expect("catalog");
        let catalog = source.catalog().expect("supplied");
        assert_eq!(catalog.len(), 1);
        assert_eq!((catalog[0].rows, catalog[0].translatable), (2, 2));
        let path = catalog_path(cache.path(), &source);
        assert!(path.is_file());
        assert_eq!(read_catalog(&path, &source), Some(catalog.to_vec()));

        fs::write(&path, b"not json").expect("damage");
        let reopened = GameSource::open(game.path(), SourceLanguage::English).expect("source");
        load_catalog_with_cache(cache.path(), &reopened).expect("recount");
        assert_eq!(reopened.catalog(), Some(catalog));
    }
}
