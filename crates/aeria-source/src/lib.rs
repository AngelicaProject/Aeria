//! The installed game as Aeria's translation source.
//!
//! [`GameSource`] opens a game installation in one source language and
//! reads its sheets on demand. Every source fact Aeria persists or checks is
//! derived here: String cell text, sheet layouts, translation permission, and
//! row keys. See `docs/architecture/source.md`.

#![forbid(unsafe_code)]

mod dialogue;
mod glyphs;
mod script;
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
use script::ResolvedCutscene;
pub use script::{
    Availability, Choice, ChoiceKind, ChoiceOption, ChunkError, Comparison, Condition, Constant,
    CutsceneError, FlowNode, Guard, Operand, OptionLabel, QuestReference, QuestScript, SceneFlow,
    Test, cutscene_keys,
};
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

/// A sheet's String cells in another client language: see
/// [`GameSource::sheet_in_language`].
pub struct SheetInLanguage {
    sheet: Arc<SourceSheet>,
    strings: Option<sheet::StringRows>,
}

impl SheetInLanguage {
    /// The macro text of a cell, or `None` when that language has no such
    /// cell or its sheet does not match this one.
    #[must_use]
    pub fn text(&self, row: u32, subrow: u16, column: u32) -> Option<String> {
        self.strings
            .as_ref()
            .and_then(|strings| self.sheet.text_in(strings, row, subrow, column))
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

    /// A quest script is not a compiled script Aeria reads.
    #[error("the script of {sheet} cannot be read: {source}")]
    Script { sheet: String, source: ChunkError },

    /// A cutscene a quest plays is not a cutscene file Aeria reads.
    #[error("the cutscene {path} cannot be read: {source}")]
    Cutscene { path: String, source: CutsceneError },
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
    variables: Mutex<HashMap<String, Arc<QuestVariables>>>,
    cutscene_index: Mutex<Option<Arc<CutsceneIndex>>>,
    /// Where quests' scripts play each `Cutscene` row asked about so far.
    cutscene_plays: Mutex<HashMap<u32, Vec<CutscenePlay>>>,
    speakers: Mutex<Option<Arc<SpeakerIndex>>>,
}

/// Quest sheet IDs and the `Quest` row that names each one.
type QuestIndex = HashMap<String, QuestEntry>;

/// The script variables of each row of `Quest` or `QuestBattle`, such as
/// `CUT_SCENE_01 = 10`.
type QuestVariables = HashMap<(u32, u16), Vec<(String, u32)>>;

/// Every cutscene file's text keys, by the dialogue sheet they belong to.
type CutsceneIndex = HashMap<String, Vec<CutsceneLines>>;

/// A cutscene file and the keys of one dialogue sheet it names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CutsceneLines {
    /// The cutscene's `Cutscene` row.
    pub row: u32,
    /// The cutscene's path, such as `ffxiv/clsarc/clsarc00110/clsarc00110`.
    pub path: String,
    /// The sheet's keys the file names, in the sheet's row order.
    pub keys: Vec<String>,
}

/// A scene or handler of a quest's scripts that plays a cutscene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CutscenePlay {
    /// The quest's sheet, such as `quest/047/AktKmm103_04753`.
    pub quest: String,
    /// The quest's name.
    pub name: Option<String>,
    /// As in [`SceneFlow`]: the scene number, or the handler's name, and the
    /// battle script it belongs to.
    pub scene: Option<u32>,
    pub handler: Option<String>,
    pub script: Option<String>,
}

/// Script suffixes of a quest's battles: `ClsRog250_00148` has its battle
/// script at `ClsRog250Btl_00148`, and a second one at `ClsRog250Btl2_00148`.
const BATTLE_SUFFIXES: [&str; 9] = [
    "Btl", "Btl2", "Btl3", "Btl4", "Btl5", "Btl6", "Btl7", "Btl8", "Btl9",
];

/// A quest sheet's `Quest` row.
struct QuestEntry {
    sheet: String,
    row: (u32, u16),
    /// The text of the row's first non-empty translatable cell.
    name: Option<String>,
}

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
            variables: Mutex::new(HashMap::new()),
            cutscene_index: Mutex::new(None),
            cutscene_plays: Mutex::new(HashMap::new()),
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
    /// A sheet in another client language, read once, to look up many of its
    /// cells: faster than [`Self::cell_in_other_languages`] for every cell of
    /// a sheet, and without the shared cache several threads would take from
    /// each other.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn sheet_in_language(
        &self,
        sheet: &Arc<SourceSheet>,
        language: SourceLanguage,
    ) -> Result<SheetInLanguage, SourceError> {
        Ok(SheetInLanguage {
            sheet: Arc::clone(sheet),
            strings: sheet.strings_in(&self.game, language)?,
        })
    }

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

    /// The traced dialogue flow of a quest sheet's script,
    /// `game_script/<sheet>.luab` (see [`QuestScript`]); `None` for any other
    /// sheet, or a quest without a script.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read or the script is not
    /// a compiled script Aeria reads.
    pub fn quest_script(&self, sheet: &str) -> Result<Option<QuestScript>, SourceError> {
        if DialogueKind::of(sheet) != Some(DialogueKind::Quest)
            || !self.sheet_names.iter().any(|name| name == sheet)
        {
            return Ok(None);
        }
        let Some(data) = self.game.file(&format!("game_script/{sheet}.luab"))? else {
            return Ok(None);
        };
        let mut script = QuestScript::read(&data).map_err(|source| SourceError::Script {
            sheet: sheet.to_owned(),
            source,
        })?;
        let variables = self.quest_row_variables(sheet)?;
        self.resolve_cutscenes(sheet, &mut script, &variables)?;
        self.resolve_quests(sheet, &mut script, &variables)?;
        // The quest's battles have scripts of their own, named after it,
        // whose cutscene variables are in the `QuestBattle` rows the quest's
        // `QUESTBATTLE…` variables name.
        let battle_variables = self.battle_variables(&variables);
        if let Some((folder, id)) = sheet.rsplit_once('/')
            && let Some((base, number)) = id.rsplit_once('_')
        {
            for suffix in BATTLE_SUFFIXES {
                let name = format!("{base}{suffix}");
                let path = format!("game_script/{folder}/{name}_{number}.luab");
                let Some(data) = self.game.file(&path)? else {
                    continue;
                };
                let mut battle =
                    QuestScript::read(&data).map_err(|source| SourceError::Script {
                        sheet: path.clone(),
                        source,
                    })?;
                self.resolve_cutscenes(sheet, &mut battle, &battle_variables)?;
                self.resolve_quests(sheet, &mut battle, &variables)?;
                for scene in &mut battle.scenes {
                    scene.script = Some(name.clone());
                }
                script.scenes.extend(battle.scenes);
            }
        }
        Ok(Some(script))
    }

    /// The script variables of a quest sheet's `Quest` row.
    fn quest_row_variables(&self, sheet: &str) -> Result<Vec<(String, u32)>, SourceError> {
        let Some(row) = self.quest_row(sheet)? else {
            return Ok(Vec::new());
        };
        Ok(self
            .variables("Quest")
            .get(&row)
            .cloned()
            .unwrap_or_default())
    }

    /// The script variables of the `QuestBattle` rows that a quest's
    /// `QUESTBATTLE…` variables name. A name with different values in two
    /// rows is left out, since which row a battle script uses is unknown.
    fn battle_variables(&self, quest: &[(String, u32)]) -> Vec<(String, u32)> {
        let battles = self.variables("QuestBattle");
        let mut merged: Vec<(String, u32)> = Vec::new();
        let mut conflicts: Vec<String> = Vec::new();
        for (_, row) in quest
            .iter()
            .filter(|(name, _)| name.starts_with("QUESTBATTLE"))
        {
            for (name, value) in battles.get(&(*row, 0)).into_iter().flatten() {
                match merged.iter().find(|(known, _)| known == name) {
                    Some((_, known)) if known != value => conflicts.push(name.clone()),
                    Some(_) => {}
                    None => merged.push((name.clone(), *value)),
                }
            }
        }
        merged.retain(|(name, _)| !conflicts.contains(name));
        merged
    }

    /// Every cutscene file that names keys of a dialogue sheet, with those
    /// keys in the sheet's row order, in `Cutscene` row order. Reading every
    /// cutscene file takes a few seconds on up to `threads` threads the first
    /// time; the index is kept in memory. A file that is missing or is not a
    /// cutscene file is left out of the index.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn cutscenes_naming(
        &self,
        sheet: &str,
        threads: usize,
    ) -> Result<Vec<CutsceneLines>, SourceError> {
        let index = self.cutscene_index(threads)?;
        let Some(found) = index.get(sheet) else {
            return Ok(Vec::new());
        };
        let rows: HashMap<String, usize> = self
            .dialogue(sheet)?
            .map(|dialogue| {
                dialogue
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| (line.key.to_ascii_uppercase(), index))
                    .collect()
            })
            .unwrap_or_default();
        Ok(found
            .iter()
            .map(|cutscene| {
                let mut keys = cutscene.keys.clone();
                keys.sort_by_key(|key| {
                    rows.get(&key.to_ascii_uppercase())
                        .copied()
                        .unwrap_or(usize::MAX)
                });
                CutsceneLines {
                    keys,
                    ..cutscene.clone()
                }
            })
            .collect())
    }

    /// The quests whose scripts play each of the `Cutscene` rows, with the
    /// scene or handler that plays it, in sheet-name order. Quests whose
    /// `Quest` row, or a `QuestBattle` row it names, holds one of the rows
    /// are traced to find where their scripts play it; a quest whose script
    /// does not play it is left out. Results are kept per row for the source.
    ///
    /// # Errors
    ///
    /// Returns an error when the game cannot be read. A quest whose script
    /// cannot be read is left out.
    pub fn cutscene_plays(
        &self,
        rows: &[u32],
    ) -> Result<HashMap<u32, Vec<CutscenePlay>>, SourceError> {
        let cached = |row: &u32| {
            self.cutscene_plays
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(row)
        };
        let missing: Vec<u32> = rows.iter().filter(|row| !cached(row)).copied().collect();
        if !missing.is_empty() {
            let found = self.find_cutscene_plays(&missing)?;
            let mut cache = self
                .cutscene_plays
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for row in missing {
                cache.insert(row, found.get(&row).cloned().unwrap_or_default());
            }
        }
        let cache = self
            .cutscene_plays
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(rows
            .iter()
            .filter_map(|row| {
                let plays = cache.get(row).filter(|plays| !plays.is_empty())?;
                Some((*row, plays.clone()))
            })
            .collect())
    }

    /// [`Self::cutscene_plays`] without the cache.
    fn find_cutscene_plays(
        &self,
        rows: &[u32],
    ) -> Result<HashMap<u32, Vec<CutscenePlay>>, SourceError> {
        let mut plays: HashMap<u32, Vec<CutscenePlay>> = HashMap::new();
        if rows.is_empty() {
            return Ok(plays);
        }
        let holds =
            |variables: &[(String, u32)]| variables.iter().any(|(_, value)| rows.contains(value));
        let battles: HashSet<u32> = self
            .variables("QuestBattle")
            .iter()
            .filter(|(_, variables)| holds(variables))
            .map(|((row, _), _)| *row)
            .collect();
        let quests: HashSet<u32> = self
            .variables("Quest")
            .iter()
            .filter(|(_, variables)| {
                holds(variables)
                    || variables.iter().any(|(name, value)| {
                        name.starts_with("QUESTBATTLE") && battles.contains(value)
                    })
            })
            .map(|((row, _), _)| *row)
            .collect();
        let index = self.quest_index()?;
        let mut candidates: Vec<&QuestEntry> = index
            .values()
            .filter(|entry| entry.row.1 == 0 && quests.contains(&entry.row.0))
            .collect();
        candidates.sort_unstable_by(|left, right| left.sheet.cmp(&right.sheet));
        for entry in candidates {
            let script = match self.quest_script(&entry.sheet) {
                Ok(Some(script)) => script,
                Ok(None) | Err(SourceError::Script { .. } | SourceError::Cutscene { .. }) => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            for (scene, row) in script.cutscene_rows() {
                if !rows.contains(&row) {
                    continue;
                }
                let scene = &script.scenes[scene];
                plays.entry(row).or_default().push(CutscenePlay {
                    quest: entry.sheet.clone(),
                    name: entry.name.clone(),
                    scene: scene.scene,
                    handler: scene.handler.clone(),
                    script: scene.script.clone(),
                });
            }
        }
        Ok(plays)
    }

    fn cutscene_index(&self, threads: usize) -> Result<Arc<CutsceneIndex>, SourceError> {
        let mut index = self
            .cutscene_index
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = index.as_ref() {
            return Ok(Arc::clone(index));
        }
        let SheetLookup::Present(cutscenes) = self.sheet("Cutscene")? else {
            return Ok(Arc::clone(index.insert(Arc::new(CutsceneIndex::new()))));
        };
        let files: Vec<(u32, String)> = cutscenes
            .rows()
            .iter()
            .filter_map(|row| {
                let path = cutscenes
                    .cells(row)
                    .find(|cell| !cell.bytes.is_empty())?
                    .text();
                Some((row.row_id, path))
            })
            .collect();
        let mut sheets_by_id: HashMap<String, &str> = HashMap::new();
        for name in &self.sheet_names {
            if DialogueKind::of(name).is_some() {
                sheets_by_id.insert(sheet_id(name).to_ascii_uppercase(), name);
            }
        }
        let read = |chunk: &[(u32, String)]| -> Result<Vec<(String, CutsceneLines)>, SourceError> {
            let mut found = Vec::new();
            for (row, path) in chunk {
                let Some(data) = self.game.file(&format!("cut/{path}.cutb"))? else {
                    continue;
                };
                let Ok(keys) = cutscene_keys(&data) else {
                    continue;
                };
                let mut by_sheet: Vec<(String, Vec<String>)> = Vec::new();
                for key in keys {
                    let Some(sheet) = sheet_of_key(&sheets_by_id, &key) else {
                        continue;
                    };
                    match by_sheet.iter_mut().find(|(known, _)| known == sheet) {
                        Some((_, keys)) => keys.push(key),
                        None => by_sheet.push((sheet.to_owned(), vec![key])),
                    }
                }
                found.extend(by_sheet.into_iter().map(|(sheet, keys)| {
                    (
                        sheet,
                        CutsceneLines {
                            row: *row,
                            path: path.clone(),
                            keys,
                        },
                    )
                }));
            }
            Ok(found)
        };
        let chunks: Vec<&[(u32, String)]> = files
            .chunks(files.len().div_ceil(threads.max(1)).max(1))
            .collect();
        let found: Vec<Result<Vec<(String, CutsceneLines)>, SourceError>> =
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunks
                    .iter()
                    .map(|chunk| scope.spawn(|| read(chunk)))
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
        let mut built = CutsceneIndex::new();
        for chunk in found {
            for (sheet, cutscene) in chunk? {
                built.entry(sheet).or_default().push(cutscene);
            }
        }
        Ok(Arc::clone(index.insert(Arc::new(built))))
    }

    /// Replaces the quest variables of quest functions, such as `QUEST0` in
    /// `IsQuestCompleted(QUEST0)`, with the quests they name: the variable's
    /// value in the quest's `Quest` row is the named quest's `Quest` row.
    /// A variable without a value stays a field.
    fn resolve_quests(
        &self,
        _sheet: &str,
        script: &mut QuestScript,
        row_variables: &[(String, u32)],
    ) -> Result<(), SourceError> {
        let names = script.quest_variables();
        if names.is_empty() || row_variables.is_empty() {
            return Ok(());
        }
        let index = self.quest_index()?;
        let by_row: HashMap<u32, &QuestEntry> = index
            .values()
            .filter(|entry| entry.row.1 == 0)
            .map(|entry| (entry.row.0, entry))
            .collect();
        let mut resolved = HashMap::new();
        for name in names {
            let Some(value) = row_variables
                .iter()
                .find(|(variable, _)| *variable == name)
                .map(|(_, value)| *value)
            else {
                continue;
            };
            let entry = by_row.get(&value);
            resolved.insert(
                name.clone(),
                QuestReference {
                    variable: name,
                    row: value,
                    name: entry.and_then(|entry| entry.name.clone()),
                    sheet: entry.map(|entry| entry.sheet.clone()),
                },
            );
        }
        script.set_quests(&resolved);
        Ok(())
    }

    /// Fills the lines of the cutscenes a quest's scenes play. A cutscene
    /// variable resolves through the quest's script variables in its `Quest`
    /// row (each `String` column is a variable name, and the `UInt32` column
    /// that follows it in the row data holds its value), the `Cutscene` row
    /// of that value, whose first String column is a path, and the file
    /// `cut/<path>.cutb`. A link that is missing leaves the cutscene without
    /// lines.
    fn resolve_cutscenes(
        &self,
        sheet: &str,
        script: &mut QuestScript,
        row_variables: &[(String, u32)],
    ) -> Result<(), SourceError> {
        let names = script.cutscene_names();
        if names.is_empty() || row_variables.is_empty() {
            return Ok(());
        }
        let SheetLookup::Present(cutscenes) = self.sheet("Cutscene")? else {
            return Ok(());
        };
        let rows: HashMap<String, usize> = self
            .dialogue(sheet)?
            .map(|dialogue| {
                dialogue
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| (line.key.to_ascii_uppercase(), index))
                    .collect()
            })
            .unwrap_or_default();
        let mut sheets_by_id: HashMap<String, &str> = HashMap::new();
        for name in &self.sheet_names {
            if DialogueKind::of(name).is_some() {
                sheets_by_id.insert(sheet_id(name).to_ascii_uppercase(), name);
            }
        }
        let mut resolved = HashMap::new();
        for name in names {
            let Some(value) = row_variables
                .iter()
                .find(|(variable, _)| *variable == name)
                .map(|(_, value)| *value)
            else {
                continue;
            };
            let Some(path) = cutscenes
                .row(value, 0)
                .and_then(|row| cutscenes.cells(row).find(|cell| !cell.bytes.is_empty()))
                .map(|cell| cell.text())
            else {
                continue;
            };
            let file = format!("cut/{path}.cutb");
            let Some(data) = self.game.file(&file)? else {
                continue;
            };
            let keys = cutscene_keys(&data).map_err(|source| SourceError::Cutscene {
                path: file.clone(),
                source,
            })?;
            let mut lines: Vec<(usize, String)> = Vec::new();
            let mut others: Vec<String> = Vec::new();
            for key in keys {
                if let Some(index) = rows.get(&key.to_ascii_uppercase()) {
                    lines.push((*index, key));
                } else if let Some(other) = sheet_of_key(&sheets_by_id, &key)
                    && other != sheet
                    && !others.iter().any(|known| known == other)
                {
                    others.push(other.to_owned());
                }
            }
            lines.sort_unstable();
            others.sort_unstable();
            resolved.insert(
                name,
                ResolvedCutscene {
                    row: value,
                    path,
                    lines: lines.into_iter().map(|(_, key)| key).collect(),
                    sheets: others,
                },
            );
        }
        script.set_cutscenes(&resolved);
        Ok(())
    }

    fn variables(&self, sheet: &str) -> Arc<QuestVariables> {
        let mut variables = self
            .variables
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            variables
                .entry(sheet.to_owned())
                .or_insert_with(|| Arc::new(read_variables(&self.game, sheet, self.language))),
        )
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
        Ok(self
            .quest_index()?
            .get(sheet_id(sheet))
            .map(|entry| entry.row))
    }

    /// The other quest sheets whose `Quest` row has the same name, such as
    /// the versions of a class quest the game gives depending on the
    /// player's starting class, in sheet-name order. The name is the source
    /// text of the row's first non-empty translatable cell, compared
    /// exactly; a quest without a row or a name has no versions.
    ///
    /// # Errors
    ///
    /// Returns an error when a game file cannot be read.
    pub fn quest_versions(&self, sheet: &str) -> Result<Vec<String>, SourceError> {
        if DialogueKind::of(sheet) != Some(DialogueKind::Quest) {
            return Ok(Vec::new());
        }
        let index = self.quest_index()?;
        let Some(name) = index
            .get(sheet_id(sheet))
            .filter(|entry| entry.sheet == sheet)
            .and_then(|entry| entry.name.as_deref())
        else {
            return Ok(Vec::new());
        };
        let mut versions: Vec<String> = index
            .values()
            .filter(|entry| entry.sheet != sheet && entry.name.as_deref() == Some(name))
            .map(|entry| entry.sheet.clone())
            .collect();
        versions.sort_unstable();
        Ok(versions)
    }

    fn quest_index(&self) -> Result<Arc<QuestIndex>, SourceError> {
        let mut quests = self.quests.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(match quests.as_ref() {
            Some(index) => Arc::clone(index),
            None => Arc::clone(quests.insert(Arc::new(self.read_quest_index()?))),
        })
    }

    fn read_quest_index(&self) -> Result<QuestIndex, SourceError> {
        // Quest sheet IDs, and the sheet of each; an ID in two sheets names
        // neither.
        let mut sheets: HashMap<&str, Option<&str>> = HashMap::new();
        for name in &self.sheet_names {
            if DialogueKind::of(name) == Some(DialogueKind::Quest) {
                sheets
                    .entry(sheet_id(name))
                    .and_modify(|sheet| *sheet = None)
                    .or_insert(Some(name.as_str()));
            }
        }
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
                if sheets.contains_key(text.as_str()) {
                    let found = rows.entry(text).or_default();
                    if !found.contains(&(row.row_id, row.subrow_id)) {
                        found.push((row.row_id, row.subrow_id));
                    }
                }
            }
        }
        Ok(rows
            .into_iter()
            .filter_map(|(id, rows)| {
                let [row] = rows.as_slice() else {
                    return None;
                };
                let sheet = sheets.get(id.as_str()).copied().flatten()?.to_owned();
                let name = quest
                    .row(row.0, row.1)
                    .and_then(|source| {
                        quest
                            .cells(source)
                            .find(|cell| cell.translatable && !cell.bytes.is_empty())
                    })
                    .map(|cell| cell.text());
                Some((
                    id,
                    QuestEntry {
                        sheet,
                        row: *row,
                        name,
                    },
                ))
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

/// The dialogue sheet a text key belongs to: the one whose ID follows
/// `TEXT_` in the key, ignoring case, taking the longest ID that matches.
fn sheet_of_key<'a>(sheets_by_id: &HashMap<String, &'a str>, key: &str) -> Option<&'a str> {
    let rest = key.get(5..).filter(|_| {
        key.get(..5)
            .is_some_and(|head| head.eq_ignore_ascii_case("TEXT_"))
    })?;
    rest.match_indices('_')
        .filter_map(|(end, _)| sheets_by_id.get(&rest[..end].to_ascii_uppercase()).copied())
        .next_back()
}

/// The script variables of every row of `Quest` or `QuestBattle`: each
/// non-empty String column paired with the `UInt32` column whose data starts
/// 4 bytes after it. A sheet that cannot be read has none.
fn read_variables(game: &GameData, name: &str, language: SourceLanguage) -> QuestVariables {
    let Ok(sheet) = excel::read_sheet(game, name, language.excel())
        .or_else(|_| excel::read_sheet(game, name, excel::Language::None))
    else {
        return QuestVariables::new();
    };
    let pairs: Vec<(usize, usize)> = sheet
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.kind == excel::ColumnKind::String)
        .filter_map(|(index, column)| {
            let value = sheet.columns.iter().position(|other| {
                other.kind == excel::ColumnKind::UInt32 && other.offset == column.offset + 4
            })?;
            Some((index, value))
        })
        .collect();
    sheet
        .rows()
        .iter()
        .filter_map(|row| {
            let variables: Vec<(String, u32)> = pairs
                .iter()
                .filter_map(|(name, value)| {
                    let name = row.string(*name).ok().filter(|name| {
                        !name.is_empty()
                            && name
                                .iter()
                                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    })?;
                    let bytes: [u8; 4] = row.value(*value).ok()?.as_slice().try_into().ok()?;
                    Some((
                        String::from_utf8_lossy(name).into_owned(),
                        u32::from_le_bytes(bytes),
                    ))
                })
                .collect();
            (!variables.is_empty()).then_some(((row.row_id, row.subrow_id), variables))
        })
        .collect()
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
