//! The installed game as Aeria's translation source.
//!
//! [`GameSource`] opens a game installation in one source language and
//! reads its sheets on demand. Every source fact Aeria persists or checks is
//! derived here: String cell text, sheet layouts, translation permission, and
//! row keys. See `docs/architecture/source.md`.

#![forbid(unsafe_code)]

mod dialogue;
mod glyphs;
mod sheet;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, PoisonError};

pub use aeria_sqpack::excel::Variant as SheetVariant;
use aeria_sqpack::excel::{self, Language};
use aeria_sqpack::{GameData, SqPackError};
use thiserror::Error;

pub use aeria_core::{GameVersion, GameVersionError, LayoutHash};
pub use dialogue::{Dialogue, DialogueKind, DialogueLine, LineRole, line_role, sheet_id};
pub use glyphs::{FontGlyph, FontGlyphs, Icon};
pub use sheet::{
    MIN_KEYED_ROWS, RowKeys, SourceCell, SourceRow, SourceSheet, StringColumn, Unavailable,
    layout_hash,
};

/// How many recently read sheets a source keeps in memory.
const CACHED_SHEETS: usize = 16;

/// How many sheets read in another language a source keeps in memory: the
/// other three client languages of the last two sheets.
const CACHED_OTHER_LANGUAGE_SHEETS: usize = 6;

/// A sheet read in another client language; `None` when it cannot be
/// matched to the source sheet.
type OtherLanguageSheet = Arc<Option<sheet::StringRows>>;

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
    other_languages: Mutex<VecDeque<((String, SourceLanguage), OtherLanguageSheet)>>,
    catalog: std::sync::OnceLock<Arc<Vec<SheetSummary>>>,
    ui_colors: std::sync::OnceLock<Vec<(u32, u32)>>,
    icons: std::sync::OnceLock<Option<(Vec<u8>, Vec<u8>)>>,
    quests: Mutex<Option<Arc<QuestIndex>>>,
    speakers: Mutex<Option<Arc<SpeakerIndex>>>,
}

/// Quest sheet IDs and the `Quest` row that names each one.
type QuestIndex = HashMap<String, (u32, u16)>;

/// Speech lines by speaker label: `(sheet index, row, subrow)` in sheet-name
/// and row order.
type SpeakerIndex = BTreeMap<String, Vec<(u32, u32, u16)>>;

/// One speech line: `(speaker, sheet index, row, subrow)`.
type SpeechLine = (String, u32, u32, u16);

/// One row of a sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowAddress {
    pub sheet: String,
    pub row_id: u32,
    pub subrow_id: u16,
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
            other_languages: Mutex::new(VecDeque::new()),
            catalog: std::sync::OnceLock::new(),
            ui_colors: std::sync::OnceLock::new(),
            icons: std::sync::OnceLock::new(),
            quests: Mutex::new(None),
            speakers: Mutex::new(None),
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
    /// such as a class name.
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

    /// The text of one String cell in every client language other than the
    /// source language, in [`SourceLanguage::ALL`] order: context for a
    /// translator, never a source fact. A language's text is `None` when
    /// the sheet cannot be read in it, has other String columns there, or
    /// lacks the row; every text is `None` when the sheet itself cannot be
    /// read.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn cell_in_other_languages(
        &self,
        sheet: &str,
        row: u32,
        subrow: u16,
        column: u32,
    ) -> Result<Vec<(SourceLanguage, Option<String>)>, SourceError> {
        let own = match self.sheet(sheet)? {
            SheetLookup::Present(own) => Some(own),
            SheetLookup::Missing | SheetLookup::Unavailable(_) => None,
        };
        let mut texts = Vec::new();
        for language in SourceLanguage::ALL {
            if language == self.language {
                continue;
            }
            let text = match &own {
                Some(own) => {
                    let strings = self.other_language_sheet(own, language)?;
                    strings
                        .as_ref()
                        .as_ref()
                        .and_then(|strings| own.text_in(strings, row, subrow, column))
                }
                None => None,
            };
            texts.push((language, text));
        }
        Ok(texts)
    }

    /// Reads `own` in another language, or takes it from memory.
    fn other_language_sheet(
        &self,
        own: &SourceSheet,
        language: SourceLanguage,
    ) -> Result<OtherLanguageSheet, SourceError> {
        let key = (own.name().to_owned(), language);
        {
            let mut cache = self
                .other_languages
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(entry) = cache
                .iter()
                .position(|(cached, _)| *cached == key)
                .and_then(|position| cache.remove(position))
            {
                let strings = Arc::clone(&entry.1);
                cache.push_front(entry);
                return Ok(strings);
            }
        }
        let strings = Arc::new(own.strings_in(&self.game, language)?);
        let mut cache = self
            .other_languages
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cache.push_front((key, Arc::clone(&strings)));
        cache.truncate(CACHED_OTHER_LANGUAGE_SHEETS);
        Ok(strings)
    }

    /// The private use glyphs of the game font, such as `U+E03C`, the
    /// high-quality mark: symbols game text writes as characters that only
    /// the game font draws. `None` when the font cannot be read.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn private_glyphs(&self) -> Result<Option<FontGlyphs>, SourceError> {
        let Some(fdt) = self.game.file(glyphs::GLYPH_FONT)? else {
            return Ok(None);
        };
        let mut failure = None;
        let result = glyphs::private_glyphs(&fdt, |page| {
            match self.game.file(&format!("common/font/font{}.tex", page + 1)) {
                Ok(file) => file,
                Err(error) => {
                    failure = Some(error);
                    None
                }
            }
        });
        match failure {
            Some(error) => Err(error.into()),
            None => Ok(result),
        }
    }

    /// The inline icon an `<icon>` macro shows, at double size, as the game
    /// shows it with a keyboard or an Xbox controller. `None` for an id the
    /// game has no icon for.
    #[must_use]
    pub fn icon(&self, id: u32) -> Option<Icon> {
        let files = self.icons.get_or_init(|| {
            let table = self.game.file(glyphs::ICON_TABLE).ok()??;
            let texture = self.game.file(glyphs::ICON_TEXTURE).ok()??;
            Some((table, texture))
        });
        let (table, texture) = files.as_ref()?;
        glyphs::icon(table, texture, id)
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

    /// The dialogue of a quest or cutscene sheet (see [`Dialogue::of`]);
    /// `None` for any other sheet, or one without row keys.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn dialogue(&self, sheet: &str) -> Result<Option<Dialogue>, SourceError> {
        if DialogueKind::of(sheet).is_none() {
            return Ok(None);
        }
        Ok(match self.sheet(sheet)? {
            SheetLookup::Present(sheet) => Dialogue::of(&sheet),
            SheetLookup::Missing | SheetLookup::Unavailable(_) => None,
        })
    }

    /// The `Quest` row of a quest sheet: the only row with a String cell
    /// that is not translatable and holds the sheet's ID, such as
    /// `ManFst004_00124` for `quest/001/ManFst004_00124`. `None` when no row
    /// or more than one row holds it. `Quest` is read once.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn quest_row(&self, sheet: &str) -> Result<Option<(u32, u16)>, SourceError> {
        if DialogueKind::of(sheet) != Some(DialogueKind::Quest) {
            return Ok(None);
        }
        let mut quests = self.quests.lock().unwrap_or_else(PoisonError::into_inner);
        let index = match quests.as_ref() {
            Some(index) => Arc::clone(index),
            None => Arc::clone(quests.insert(Arc::new(self.read_quest_index()?))),
        };
        drop(quests);
        Ok(index.get(sheet_id(sheet)).copied())
    }

    fn read_quest_index(&self) -> Result<QuestIndex, SourceError> {
        let ids: HashSet<&str> = self
            .sheet_names
            .iter()
            .filter(|name| DialogueKind::of(name) == Some(DialogueKind::Quest))
            .map(|name| sheet_id(name))
            .collect();
        let SheetLookup::Present(quest) = self.sheet("Quest")? else {
            return Ok(QuestIndex::new());
        };
        let mut rows: HashMap<String, Vec<(u32, u16)>> = HashMap::new();
        for row in quest.rows() {
            for cell in quest.cells(row) {
                if cell.translatable || cell.bytes.is_empty() {
                    continue;
                }
                let text = cell.text();
                if ids.contains(text.as_str()) {
                    let found = rows.entry(text).or_default();
                    if !found.contains(&(row.row_id, row.subrow_id)) {
                        found.push((row.row_id, row.subrow_id));
                    }
                }
            }
        }
        Ok(rows
            .into_iter()
            .filter_map(|(id, rows)| match rows.as_slice() {
                [row] => Some((id, *row)),
                _ => None,
            })
            .collect())
    }

    /// Speaker labels of quest and cutscene speech that contain `query`
    /// (ignoring ASCII case), with their number of lines, in label order.
    /// The first call reads every quest and cutscene sheet on up to
    /// `threads` threads.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn speakers(
        &self,
        query: &str,
        threads: usize,
    ) -> Result<Vec<(String, usize)>, SourceError> {
        let query = query.to_ascii_uppercase();
        Ok(self
            .speaker_index(threads)?
            .iter()
            .filter(|(label, _)| label.to_ascii_uppercase().contains(&query))
            .map(|(label, lines)| (label.clone(), lines.len()))
            .collect())
    }

    /// The speech lines of one speaker label in sheet-name and row order:
    /// the total and the lines from `offset`, at most `limit`. The first
    /// call reads every quest and cutscene sheet on up to `threads` threads.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn speaker_lines(
        &self,
        speaker: &str,
        offset: usize,
        limit: usize,
        threads: usize,
    ) -> Result<(usize, Vec<RowAddress>), SourceError> {
        let index = self.speaker_index(threads)?;
        let Some(lines) = index.get(speaker) else {
            return Ok((0, Vec::new()));
        };
        let page = lines
            .iter()
            .skip(offset)
            .take(limit)
            .filter_map(|&(sheet, row_id, subrow_id)| {
                Some(RowAddress {
                    sheet: self.sheet_names.get(sheet as usize)?.clone(),
                    row_id,
                    subrow_id,
                })
            })
            .collect();
        Ok((lines.len(), page))
    }

    fn speaker_index(&self, threads: usize) -> Result<Arc<SpeakerIndex>, SourceError> {
        let mut speakers = self.speakers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = speakers.as_ref() {
            return Ok(Arc::clone(index));
        }
        let sheets: Vec<usize> = (0..self.sheet_names.len())
            .filter(|&index| DialogueKind::of(&self.sheet_names[index]).is_some())
            .collect();
        let chunks: Vec<&[usize]> = sheets
            .chunks(sheets.len().div_ceil(threads.max(1)).max(1))
            .collect();
        let found: Vec<Result<Vec<SpeechLine>, SourceError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunks
                .iter()
                .map(|chunk| scope.spawn(|| self.speech_lines(chunk)))
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        });
        let mut index = SpeakerIndex::new();
        for lines in found {
            for (speaker, sheet, row_id, subrow_id) in lines? {
                index
                    .entry(speaker)
                    .or_default()
                    .push((sheet, row_id, subrow_id));
            }
        }
        Ok(Arc::clone(speakers.insert(Arc::new(index))))
    }

    /// The speech lines of some sheets, as `(speaker, sheet index, row,
    /// subrow)`, in the order given.
    fn speech_lines(&self, sheets: &[usize]) -> Result<Vec<SpeechLine>, SourceError> {
        let mut lines = Vec::new();
        for &index in sheets {
            let SheetLookup::Present(sheet) = self.read_sheet(&self.sheet_names[index])? else {
                continue;
            };
            let Some(dialogue) = Dialogue::of(&sheet) else {
                continue;
            };
            let sheet_index = u32::try_from(index).unwrap_or(u32::MAX);
            for line in dialogue.lines {
                if let LineRole::Speech { speaker } = line.role {
                    lines.push((speaker, sheet_index, line.row_id, line.subrow_id));
                }
            }
        }
        Ok(lines)
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
