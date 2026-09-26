//! The installed game as Aeria's translation source.
//!
//! [`GameSource`] opens a game installation in one source language and
//! reads its sheets on demand. Every source fact Aeria persists or checks is
//! derived here: String cell text, sheet layouts, translation permission, and
//! row keys. See `docs/architecture/source.md`.

#![forbid(unsafe_code)]

mod sheet;

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, PoisonError};

pub use aeria_sqpack::excel::Variant as SheetVariant;
use aeria_sqpack::excel::{self, Language};
use aeria_sqpack::{GameData, SqPackError};
use thiserror::Error;

pub use aeria_core::{GameVersion, GameVersionError, LayoutHash};
pub use sheet::{
    MIN_KEYED_ROWS, RowKeys, SourceCell, SourceRow, SourceSheet, StringColumn, Unavailable,
    layout_hash,
};

/// How many recently read sheets a source keeps in memory.
const CACHED_SHEETS: usize = 16;

/// A source language: one of the global client languages.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceLanguage {
    Japanese,
    English,
    German,
    French,
}

impl SourceLanguage {
    /// Every source language; also the evidence languages of translation
    /// permission.
    pub const ALL: [Self; 4] = [Self::Japanese, Self::English, Self::German, Self::French];

    /// The language code, such as `en`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Japanese => "ja",
            Self::English => "en",
            Self::German => "de",
            Self::French => "fr",
        }
    }

    const fn excel(self) -> Language {
        match self {
            Self::Japanese => Language::Japanese,
            Self::English => Language::English,
            Self::German => Language::German,
            Self::French => Language::French,
        }
    }
}

impl fmt::Display for SourceLanguage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// A language code that is not a source language.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{0:?} is not a source language; use ja, en, de, or fr")]
pub struct SourceLanguageError(pub String);

impl FromStr for SourceLanguage {
    type Err = SourceLanguageError;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == code)
            .ok_or_else(|| SourceLanguageError(code.to_owned()))
    }
}

/// Errors from reading the game.
#[derive(Debug, Error)]
pub enum SourceError {
    /// The folder is not a game installation, or a game file could not be
    /// read.
    #[error(transparent)]
    Game(#[from] SqPackError),

    /// The game version file could not be read.
    #[error("could not read the game version {path}: {source}")]
    VersionFile {
        path: PathBuf,
        source: std::io::Error,
    },

    /// The game version file does not hold a version.
    #[error("the game version in {path} is invalid: {source}")]
    InvalidVersion {
        path: PathBuf,
        source: GameVersionError,
    },
}

/// The result of looking up a sheet.
#[derive(Clone, Debug)]
pub enum SheetLookup {
    /// The sheet is not in the game's sheet list.
    Missing,
    /// The sheet is listed but cannot be read.
    Unavailable(Unavailable),
    /// The sheet.
    Present(Arc<SourceSheet>),
}

/// One sheet of the game catalog with its size and translatable cells.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetSummary {
    pub name: String,
    /// Rows and subrows; `0` for a sheet that cannot be read.
    pub rows: usize,
    /// Translatable String cells; `0` for a sheet that cannot be read.
    pub translatable: usize,
    /// The sheet is listed but cannot be read.
    pub unavailable: bool,
}

/// An installed game opened in one source language.
pub struct GameSource {
    game_path: PathBuf,
    game: GameData,
    language: SourceLanguage,
    version: GameVersion,
    sheet_names: Vec<String>,
    cache: Mutex<VecDeque<(String, Arc<SourceSheet>)>>,
    catalog: std::sync::OnceLock<Arc<Vec<SheetSummary>>>,
    ui_colors: std::sync::OnceLock<Vec<(u32, u32)>>,
}

impl fmt::Debug for GameSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GameSource")
            .field("game_path", &self.game_path)
            .field("language", &self.language)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl GameSource {
    /// Opens the game at `game_path`, the folder that contains
    /// `game/sqpack`, and reads its version and sheet list.
    ///
    /// # Errors
    ///
    /// Returns an error when the folder is not a game installation or its
    /// version file or sheet list cannot be read.
    pub fn open(
        game_path: impl AsRef<Path>,
        language: SourceLanguage,
    ) -> Result<Self, SourceError> {
        let game_path = game_path.as_ref().to_owned();
        let game = GameData::open(&game_path)?;
        let version = read_version(&game_path)?;
        let mut sheet_names = excel::sheet_names(&game)?;
        sheet_names.sort_unstable();
        Ok(Self {
            game_path,
            game,
            language,
            version,
            sheet_names,
            cache: Mutex::new(VecDeque::new()),
            catalog: std::sync::OnceLock::new(),
            ui_colors: std::sync::OnceLock::new(),
        })
    }

    /// The game folder.
    #[must_use]
    pub fn game_path(&self) -> &Path {
        &self.game_path
    }

    /// The source language.
    #[must_use]
    pub const fn language(&self) -> SourceLanguage {
        self.language
    }

    /// The game version read when the source was opened.
    #[must_use]
    pub const fn version(&self) -> &GameVersion {
        &self.version
    }

    /// Reads the game version again, to find out whether the game was
    /// patched while the source was open.
    ///
    /// # Errors
    ///
    /// Returns an error when the version file cannot be read.
    pub fn current_version(&self) -> Result<GameVersion, SourceError> {
        read_version(&self.game_path)
    }

    /// The names of all sheets, sorted bytewise.
    #[must_use]
    pub fn sheet_names(&self) -> &[String] {
        &self.sheet_names
    }

    /// Looks up a sheet, reading it when it is not in memory.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn sheet(&self, name: &str) -> Result<SheetLookup, SourceError> {
        if self
            .sheet_names
            .binary_search_by(|known| known.as_str().cmp(name))
            .is_err()
        {
            return Ok(SheetLookup::Missing);
        }
        {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(entry) = cache
                .iter()
                .position(|(cached, _)| cached == name)
                .and_then(|position| cache.remove(position))
            {
                let sheet = Arc::clone(&entry.1);
                cache.push_front(entry);
                return Ok(SheetLookup::Present(sheet));
            }
        }
        let sheet = match SourceSheet::read(&self.game, name, self.language)? {
            Ok(sheet) => Arc::new(sheet),
            Err(unavailable) => return Ok(SheetLookup::Unavailable(unavailable)),
        };
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.push_front((name.to_owned(), Arc::clone(&sheet)));
        cache.truncate(CACHED_SHEETS);
        Ok(SheetLookup::Present(sheet))
    }

    /// Reads every sheet, on up to `threads` threads, and counts its rows
    /// and translatable cells, in sheet-name order. `keep_going` is asked
    /// before each sheet; returning `false` stops with `None`.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn summarize(
        &self,
        threads: usize,
        keep_going: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<Vec<SheetSummary>>, SourceError> {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let summaries: Mutex<Vec<Option<SheetSummary>>> =
            Mutex::new(vec![None; self.sheet_names.len()]);
        let failure: Mutex<Option<SourceError>> = Mutex::new(None);
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(name) = self.sheet_names.get(index) else {
                            return;
                        };
                        if cancelled.load(std::sync::atomic::Ordering::Relaxed)
                            || failure
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .is_some()
                        {
                            return;
                        }
                        if !keep_going() {
                            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                            return;
                        }
                        let summary = match self.read_sheet(name) {
                            Ok(SheetLookup::Present(sheet)) => SheetSummary {
                                name: name.clone(),
                                rows: sheet.rows().len(),
                                translatable: sheet
                                    .rows()
                                    .iter()
                                    .map(|row| {
                                        sheet.cells(row).filter(|cell| cell.translatable).count()
                                    })
                                    .sum(),
                                unavailable: false,
                            },
                            Ok(_) => SheetSummary {
                                name: name.clone(),
                                rows: 0,
                                translatable: 0,
                                unavailable: true,
                            },
                            Err(error) => {
                                failure
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .get_or_insert(error);
                                return;
                            }
                        };
                        summaries.lock().unwrap_or_else(PoisonError::into_inner)[index] =
                            Some(summary);
                    }
                });
            }
        });
        if let Some(error) = failure.into_inner().unwrap_or_else(PoisonError::into_inner) {
            return Err(error);
        }
        if cancelled.into_inner() {
            return Ok(None);
        }
        Ok(summaries
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .into_iter()
            .collect())
    }

    /// The catalog summary of every sheet, once it was supplied with
    /// [`Self::set_catalog`].
    #[must_use]
    pub fn catalog(&self) -> Option<Arc<Vec<SheetSummary>>> {
        self.catalog.get().cloned()
    }

    /// Supplies the catalog summary, for example one counted by
    /// [`Self::summarize`] or read from a cache of the same game version.
    /// A catalog that was already supplied is kept.
    pub fn set_catalog(&self, catalog: Arc<Vec<SheetSummary>>) {
        let _ = self.catalog.set(catalog);
    }

    /// The text color of a `UIColor` row in the default (dark) interface
    /// theme, as `0xRRGGBBAA`. The sheet is read once.
    #[must_use]
    pub fn ui_color(&self, row: u32) -> Option<u32> {
        let colors = self.ui_colors.get_or_init(|| read_ui_colors(&self.game));
        colors
            .binary_search_by_key(&row, |(id, _)| *id)
            .ok()
            .map(|index| colors[index].1)
    }

    /// The macro text of a String cell of any sheet in the source language,
    /// such as an item name.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn cell_text(
        &self,
        sheet: &str,
        row: u32,
        column: u32,
    ) -> Result<Option<String>, SourceError> {
        Ok(match self.sheet(sheet)? {
            SheetLookup::Present(sheet) => sheet.cell(row, 0, column).map(|cell| cell.text()),
            SheetLookup::Missing | SheetLookup::Unavailable(_) => None,
        })
    }

    /// Reads a sheet without keeping it in memory, for scans over many
    /// sheets.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn read_sheet(&self, name: &str) -> Result<SheetLookup, SourceError> {
        if self
            .sheet_names
            .binary_search_by(|known| known.as_str().cmp(name))
            .is_err()
        {
            return Ok(SheetLookup::Missing);
        }
        Ok(match SourceSheet::read(&self.game, name, self.language)? {
            Ok(sheet) => SheetLookup::Present(Arc::new(sheet)),
            Err(unavailable) => SheetLookup::Unavailable(unavailable),
        })
    }
}

/// The first color column of every `UIColor` row, sorted by row ID. A sheet
/// that cannot be read has no colors.
fn read_ui_colors(game: &GameData) -> Vec<(u32, u32)> {
    let Ok(sheet) = excel::read_sheet(game, "UIColor", excel::Language::None) else {
        return Vec::new();
    };
    let mut colors: Vec<(u32, u32)> = sheet
        .rows()
        .iter()
        .filter_map(|row| {
            let value = row.value(0).ok()?;
            let bytes: [u8; 4] = value.as_slice().try_into().ok()?;
            Some((row.row_id, u32::from_le_bytes(bytes)))
        })
        .collect();
    colors.sort_unstable_by_key(|(id, _)| *id);
    colors.dedup_by_key(|(id, _)| *id);
    colors
}

fn read_version(game_path: &Path) -> Result<GameVersion, SourceError> {
    let path = game_path.join("game").join("ffxivgame.ver");
    let text = std::fs::read_to_string(&path).map_err(|source| SourceError::VersionFile {
        path: path.clone(),
        source,
    })?;
    text.trim()
        .parse()
        .map_err(|source| SourceError::InvalidVersion { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_parse_from_their_codes() {
        for language in SourceLanguage::ALL {
            assert_eq!(language.code().parse::<SourceLanguage>(), Ok(language));
        }
        assert!("ko".parse::<SourceLanguage>().is_err());
    }
}
