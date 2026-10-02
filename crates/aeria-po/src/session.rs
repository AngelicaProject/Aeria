//! An open project: the installed game and the files of `po/`, read and
//! written one string at a time. The files are the only state: a file is
//! read again when its size or time changed, so a change made by Git or by
//! hand is seen on the next read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use aeria_knowledge::{Knowledge, KnowledgeFile};
use aeria_source::{GameSource, GameVersion, SheetLookup, SourceError, SourceSheet};

use crate::check::check_translation;
use crate::generate::{identity_keys, identity_of, is_entry};
use crate::identity::{SheetPaths, splits};
use crate::po::PoFile;
use crate::project::{
    GAME_VERSION_FIELD, PO_DIR, ProjectError, Settings, list, read_settings, write_settings,
};

/// The size and time of a file, to tell when to read it again.
type Stamp = Option<(u64, SystemTime)>;

fn stamp(path: &Path) -> Stamp {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The translation of one string, as its entry holds it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Translation {
    /// `msgstr`; empty when the string is not translated.
    pub text: String,
    /// The source changed since the translation was written.
    pub fuzzy: bool,
    /// The translator's note (`# ` comments), lines joined with `\n`.
    pub note: Option<String>,
    /// The source the translation was written for, while it is fuzzy.
    pub previous: Option<String>,
    /// Terms a person decided do not apply to the string.
    pub term_exceptions: Vec<String>,
}

impl Translation {
    fn of(entry: &crate::po::Entry) -> Option<Self> {
        let note = (!entry.notes.is_empty()).then(|| entry.notes.join("\n"));
        (!entry.translation.is_empty()
            || entry.fuzzy
            || note.is_some()
            || !entry.term_exceptions.is_empty())
        .then(|| Self {
            text: entry.translation.clone(),
            fuzzy: entry.fuzzy,
            note,
            previous: entry.previous.clone().filter(|_| entry.fuzzy),
            term_exceptions: entry.term_exceptions.clone(),
        })
    }
}

/// The translation state of one entry, exactly as its file holds it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EntryState {
    /// `msgstr`; empty when untranslated.
    pub text: String,
    pub fuzzy: bool,
    /// The source the translation was written for, while it is fuzzy.
    pub previous: Option<String>,
    pub term_exceptions: Vec<String>,
}

impl EntryState {
    fn of(entry: &crate::po::Entry) -> Self {
        Self {
            text: entry.translation.clone(),
            fuzzy: entry.fuzzy,
            previous: entry.previous.clone(),
            term_exceptions: entry.term_exceptions.clone(),
        }
    }
}

/// What an edit does to an entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditKind {
    /// A new translation, checked as a saved one; the fuzzy mark stays, so a
    /// replacement never passes for a review.
    Replace(String),
    /// The translation is cleared, so machine translation takes the string
    /// again; the fuzzy mark and previous source go with it.
    Clear,
    /// The entry goes back to an earlier state, as the file had it.
    Restore(EntryState),
    /// A term exception is added or removed: the glossary term does not
    /// apply to the string (see [`set_term_exception`]).
    TermException { term: String, add: bool },
}

/// An edit of one entry, made only while the entry still has the
/// translation and fuzzy mark it was made from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryEdit {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub expected_text: String,
    pub expected_fuzzy: bool,
    pub kind: EditKind,
}

/// An edit that was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditDone {
    pub path: String,
    pub context: String,
    pub before: EntryState,
    pub after: EntryState,
}

impl EditDone {
    /// The edit that undoes this one while the entry is unchanged since.
    #[must_use]
    pub fn undo(&self) -> EntryEdit {
        EntryEdit {
            path: self.path.clone(),
            context: self.context.clone(),
            expected_text: self.after.text.clone(),
            expected_fuzzy: self.after.fuzzy,
            kind: EditKind::Restore(self.before.clone()),
        }
    }
}

/// Why an edit was not written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkipReason {
    /// The entry's translation changed since the edit was made.
    Changed,
    /// The file has no such entry.
    Missing,
    /// The new translation has problems.
    Invalid(Vec<crate::check::Issue>),
    /// The file breaks the PO format.
    Broken(String),
}

/// An edit that was not written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditSkipped {
    pub path: String,
    pub context: String,
    pub reason: SkipReason,
}

/// What [`Session::apply_edits`] did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EditsApplied {
    pub done: Vec<EditDone>,
    pub skipped: Vec<EditSkipped>,
}

/// The translations of one file by `msgctxt`, and its counts.
#[derive(Debug, Default)]
struct FileState {
    translations: HashMap<String, Translation>,
    sheet: Option<String>,
    entries: usize,
    translated: usize,
    fuzzy: usize,
}

impl FileState {
    fn of(file: &PoFile) -> Self {
        let mut state = Self {
            sheet: file
                .entries
                .first()
                .and_then(|entry| crate::identity::Identity::parse(&entry.context).ok())
                .map(|identity| identity.sheet),
            entries: file.entries.len(),
            ..Self::default()
        };
        for entry in &file.entries {
            if !entry.translation.is_empty() {
                state.translated += 1;
            }
            if entry.fuzzy {
                state.fuzzy += 1;
            }
            if let Some(translation) = Translation::of(entry) {
                state
                    .translations
                    .insert(entry.context.clone(), translation);
            }
        }
        state
    }
}

/// Errors of opening a project.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("the project translates from {project}, and the game is in {game}")]
    SourceLanguage { project: String, game: String },
    #[error("the project is for game version {project}, and the installed game is older ({game})")]
    GameOutdated { project: String, game: String },
    #[error(
        "the project is for game version {project}, and the installed game is {game}: update the project to it"
    )]
    UpdateRequired { project: String, game: String },
    #[error("the project's game version {0:?} is not a version")]
    Version(String),
}

/// Errors of reading or writing a string.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("the game cannot be read: {0}")]
    Source(#[from] SourceError),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("{0} is not a string of the project")]
    NotAnEntry(String),
    #[error("po/{path} line {line}: {message}; fix the file first")]
    Broken {
        path: String,
        line: usize,
        message: String,
    },
    #[error(
        "the text of {0} in the project is not the installed game's; update the project to the game"
    )]
    SourceMismatch(String),
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Invalid(Vec<crate::check::Issue>),
}

/// One string of a row, with its translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellView {
    pub column: u32,
    pub source: String,
    /// The source has no letters outside its macros, such as `...` or a
    /// number format.
    pub formatting_only: bool,
    pub translation: Option<Translation>,
}

/// A row with its strings, and its other non-empty cells as context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowView {
    pub row: u32,
    pub subrow: u16,
    pub context: Vec<(u32, String)>,
    pub cells: Vec<CellView>,
}

/// A page of rows; `next_after` continues it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    pub rows: Vec<RowView>,
    pub next_after: Option<(u32, u16)>,
}

/// How much of a sheet is translated.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SheetProgress {
    pub sheet: String,
    pub entries: usize,
    pub translated: usize,
    pub fuzzy: usize,
}

/// An open project.
pub struct Session {
    root: PathBuf,
    settings: Mutex<Settings>,
    source: Arc<GameSource>,
    paths: SheetPaths,
    files: Mutex<HashMap<String, (Stamp, Arc<FileState>)>>,
    /// Files read for the editor, by path relative to `po/`, with the stamp
    /// they had: [`Session::changed_sheets`] watches them.
    viewed: Mutex<HashMap<String, Stamp>>,
    knowledge: Mutex<Option<(Vec<Stamp>, Arc<Knowledge>)>>,
    /// Writes one at a time, so two edits of one file never race.
    writing: Mutex<()>,
    /// Counts the changes of the files: every write of the session, and
    /// every change underneath it that the caller reports with
    /// [`Session::touch`].
    revision: std::sync::atomic::AtomicU64,
}

/// The game version a project's files are for: `aeria.json`, or the header
/// of its first file for a project that does not record it there.
fn project_version(root: &Path, settings: &Settings) -> Result<String, ProjectError> {
    if !settings.game_version.is_empty() {
        return Ok(settings.game_version.clone());
    }
    let first = list(root)?.into_iter().next();
    Ok(first
        .and_then(|path| std::fs::read_to_string(root.join(PO_DIR).join(path)).ok())
        .and_then(|text| {
            PoFile::parse(&text)
                .0
                .field(GAME_VERSION_FIELD)
                .map(str::to_owned)
        })
        .unwrap_or_default())
}

impl Session {
    /// Opens a project for the installed game. Opening never writes.
    ///
    /// # Errors
    ///
    /// Returns an error when the settings cannot be read, the game is in
    /// another source language, or the game is not the version the project
    /// is for.
    pub fn open(root: &Path, source: Arc<GameSource>) -> Result<Self, OpenError> {
        let settings = read_settings(root)?;
        let game_language = source.language().code();
        if settings.source_language != game_language {
            return Err(OpenError::SourceLanguage {
                project: settings.source_language,
                game: game_language.to_owned(),
            });
        }
        let project = project_version(root, &settings)?;
        let game = source.version().to_string();
        if project != game {
            let parsed =
                GameVersion::from_str(&project).map_err(|_| OpenError::Version(project.clone()))?;
            return Err(if parsed > *source.version() {
                OpenError::GameOutdated { project, game }
            } else {
                OpenError::UpdateRequired { project, game }
            });
        }
        Ok(Self::new(root, settings, source))
    }

    fn new(root: &Path, settings: Settings, source: Arc<GameSource>) -> Self {
        let paths = SheetPaths::new(source.sheet_names().iter().map(String::as_str));
        Self {
            root: root.to_owned(),
            settings: Mutex::new(settings),
            source,
            paths,
            files: Mutex::new(HashMap::new()),
            viewed: Mutex::new(HashMap::new()),
            knowledge: Mutex::new(None),
            writing: Mutex::new(()),
            revision: std::sync::atomic::AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project settings, `aeria.json`.
    #[must_use]
    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    /// How many times the project's files changed through the session or
    /// [`Session::touch`]: a view of them is current while it is the same.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Reports a change of the files made underneath the session, such as a
    /// Git branch switch.
    pub fn touch(&self) {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// Holds every write of the session until the guard drops, for an
    /// operation that changes the files underneath it, such as a Git merge.
    pub fn hold_writes(&self) -> MutexGuard<'_, ()> {
        lock(&self.writing)
    }

    #[must_use]
    pub fn source(&self) -> &GameSource {
        &self.source
    }

    #[must_use]
    pub fn source_handle(&self) -> Arc<GameSource> {
        Arc::clone(&self.source)
    }

    /// Sets the language the project translates into.
    ///
    /// # Errors
    ///
    /// Returns an error when `aeria.json` cannot be written.
    pub fn set_target_language(&self, tag: &str) -> Result<(), ProjectError> {
        let _writing = lock(&self.writing);
        let mut settings = lock(&self.settings);
        if settings.target_language == tag {
            return Ok(());
        }
        let mut changed = settings.clone();
        tag.clone_into(&mut changed.target_language);
        write_settings(&self.root, &changed)?;
        *settings = changed;
        Ok(())
    }

    /// The path of a sheet in `po/` without the extension: its file with
    /// `.po`, or its folder when it is split by row.
    #[must_use]
    pub fn sheet_base(&self, sheet: &str) -> String {
        self.paths.base(sheet).to_owned()
    }

    /// The project knowledge, read again when a file of it changed.
    #[must_use]
    pub fn knowledge(&self) -> Arc<Knowledge> {
        let stamps: Vec<Stamp> = KnowledgeFile::ALL
            .iter()
            .map(|file| stamp(&file.path(&self.root)))
            .collect();
        let mut cached = lock(&self.knowledge);
        if let Some((seen, knowledge)) = cached.as_ref()
            && *seen == stamps
        {
            return Arc::clone(knowledge);
        }
        let knowledge = Arc::new(Knowledge::load(&self.root));
        *cached = Some((stamps, Arc::clone(&knowledge)));
        knowledge
    }

    /// The path of the file of a row, relative to `po/`.
    fn file_of(&self, sheet: &SourceSheet, row: u32) -> String {
        let highest = sheet.rows().last().map_or(0, |last| last.row_id);
        self.paths
            .file(sheet.name(), splits(sheet.name(), highest), row)
    }

    /// The state of a file, read again when it changed. A missing file has
    /// no entries.
    fn file_state(&self, path: &str) -> Result<Arc<FileState>, EditError> {
        let full = self.root.join(PO_DIR).join(path);
        let now = stamp(&full);
        if let Some((seen, state)) = lock(&self.files).get(path)
            && *seen == now
        {
            return Ok(Arc::clone(state));
        }
        let state = match std::fs::read_to_string(&full) {
            Ok(text) => Arc::new(FileState::of(&PoFile::parse(&text).0)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Arc::new(FileState::default())
            }
            Err(error) => {
                return Err(ProjectError::Io {
                    path: full,
                    source: error,
                }
                .into());
            }
        };
        lock(&self.files).insert(path.to_owned(), (now, Arc::clone(&state)));
        Ok(state)
    }

    /// Records the stamp of a file the editor shows.
    fn view(&self, path: &str) {
        let now = stamp(&self.root.join(PO_DIR).join(path));
        lock(&self.viewed).insert(path.to_owned(), now);
    }

    /// The sheets of the files the editor has shown that changed on disk
    /// since, by Git or by hand; each is reported once per change.
    #[must_use]
    pub fn changed_sheets(&self) -> Vec<String> {
        let mut changed = std::collections::BTreeSet::new();
        let mut viewed = lock(&self.viewed);
        for (path, seen) in viewed.iter_mut() {
            let now = stamp(&self.root.join(PO_DIR).join(path));
            if now != *seen {
                *seen = now;
                if let Some((_, state)) = lock(&self.files).get(path)
                    && let Some(sheet) = &state.sheet
                {
                    changed.insert(sheet.clone());
                }
            }
        }
        changed.into_iter().collect()
    }

    /// A bounded page of a sheet's rows with their translations, after the
    /// row `after` (exclusive), ordered by row ID then subrow. A row with no
    /// string of the project is left out, so a page may be shorter than
    /// `limit` and still continue. A sheet the game does not have has none.
    ///
    /// # Errors
    ///
    /// Returns an error when the game or a file cannot be read.
    pub fn page(
        &self,
        sheet_name: &str,
        after: Option<(u32, u16)>,
        limit: usize,
    ) -> Result<Page, EditError> {
        let SheetLookup::Present(sheet) = self.source.sheet(sheet_name)? else {
            return Ok(Page {
                rows: Vec::new(),
                next_after: None,
            });
        };
        let keys = identity_keys(&sheet);
        let source_rows = sheet.rows_after(after);
        let length = source_rows.len().min(limit.max(1));
        let mut rows = Vec::with_capacity(length);
        let mut states: HashMap<String, Arc<FileState>> = HashMap::new();
        for source_row in &source_rows[..length] {
            let key = keys.and_then(|keys| keys.key_of(source_row.row_id, source_row.subrow_id));
            let mut context = Vec::new();
            let mut cells = Vec::new();
            for cell in sheet.cells(source_row) {
                let text = cell.text();
                if !is_entry(&cell, keys) {
                    if !text.is_empty() {
                        context.push((cell.column, text));
                    }
                    continue;
                }
                let path = self.file_of(&sheet, source_row.row_id);
                if !states.contains_key(&path) {
                    let state = self.file_state(&path)?;
                    self.view(&path);
                    states.insert(path.clone(), state);
                }
                let state = &states[&path];
                let identity = identity_of(
                    sheet_name,
                    key,
                    source_row.row_id,
                    source_row.subrow_id,
                    cell.column,
                )
                .to_string();
                cells.push(CellView {
                    column: cell.column,
                    formatting_only: aeria_se::parse(&text).is_formatting_only(),
                    source: text,
                    translation: state.translations.get(&identity).cloned(),
                });
            }
            if !cells.is_empty() {
                rows.push(RowView {
                    row: source_row.row_id,
                    subrow: source_row.subrow_id,
                    context,
                    cells,
                });
            }
        }
        let next_after = (length < source_rows.len()).then(|| {
            let last = &source_rows[length - 1];
            (last.row_id, last.subrow_id)
        });
        Ok(Page { rows, next_after })
    }

    /// The translation of one string.
    ///
    /// # Errors
    ///
    /// Returns an error when the string is not an entry of the project, or
    /// the game or its file cannot be read.
    pub fn translation(
        &self,
        sheet_name: &str,
        row: u32,
        subrow: u16,
        column: u32,
    ) -> Result<Option<Translation>, EditError> {
        let (path, identity, _) = self.locate(sheet_name, row, subrow, column)?;
        Ok(self.file_state(&path)?.translations.get(&identity).cloned())
    }

    /// The file of a string, relative to `po/`, its `msgctxt`, and its text
    /// in the game.
    ///
    /// # Errors
    ///
    /// Returns an error when the string is not an entry of the project or
    /// the game cannot be read.
    pub fn locate(
        &self,
        sheet_name: &str,
        row: u32,
        subrow: u16,
        column: u32,
    ) -> Result<(String, String, String), EditError> {
        let name = || format!("{sheet_name}:{row}:{subrow}:{column}");
        let SheetLookup::Present(sheet) = self.source.sheet(sheet_name)? else {
            return Err(EditError::NotAnEntry(name()));
        };
        let keys = identity_keys(&sheet);
        let cell = sheet
            .cell(row, subrow, column)
            .filter(|cell| is_entry(cell, keys))
            .ok_or_else(|| EditError::NotAnEntry(name()))?;
        let key = keys.and_then(|keys| keys.key_of(row, subrow));
        let identity = identity_of(sheet_name, key, row, subrow, column).to_string();
        Ok((self.file_of(&sheet, row), identity, cell.text()))
    }

    /// The coordinate of an entry's string in the game: sheet, row, subrow,
    /// and column. `None` when the `msgctxt` is not an identity or the game
    /// has no such string.
    #[must_use]
    pub fn coordinate_of(&self, context: &str) -> Option<(String, u32, u16, u32)> {
        let identity = crate::identity::Identity::parse(context).ok()?;
        let SheetLookup::Present(sheet) = self.source.sheet(&identity.sheet).ok()? else {
            return None;
        };
        let (row, subrow) = match &identity.row {
            crate::identity::RowName::Id { row, subrow } => (*row, *subrow),
            crate::identity::RowName::Key(key) => identity_keys(&sheet)?.row_of(key)?,
        };
        Some((identity.sheet, row, subrow, identity.column))
    }

    /// Changes one entry of a file on disk and returns its translation.
    fn edit(
        &self,
        sheet_name: &str,
        row: u32,
        subrow: u16,
        column: u32,
        change: impl FnOnce(&mut crate::po::Entry) -> Result<(), EditError>,
    ) -> Result<Option<Translation>, EditError> {
        let _writing = lock(&self.writing);
        let (path, identity, game_text) = self.locate(sheet_name, row, subrow, column)?;
        let full = self.root.join(PO_DIR).join(&path);
        let text = std::fs::read_to_string(&full).map_err(|source| ProjectError::Io {
            path: full.clone(),
            source,
        })?;
        let (mut file, problems) = PoFile::parse(&text);
        if let Some(problem) = problems.first() {
            return Err(EditError::Broken {
                path,
                line: problem.line,
                message: problem.message.clone(),
            });
        }
        let entry = file
            .entries
            .iter_mut()
            .find(|entry| entry.context == identity)
            .ok_or_else(|| EditError::NotAnEntry(identity.clone()))?;
        if entry.source != game_text {
            return Err(EditError::SourceMismatch(identity));
        }
        let before = entry.clone();
        change(entry)?;
        let translation = Translation::of(entry);
        if *entry != before {
            write_atomically(&full, &file.write())?;
            self.touch();
        }
        let state = Arc::new(FileState::of(&file));
        let now = stamp(&full);
        lock(&self.files).insert(path.clone(), (now, state));
        if let Some(seen) = lock(&self.viewed).get_mut(&path) {
            *seen = now;
        }
        Ok(translation)
    }

    /// Sets the translation of a string; an empty text leaves it
    /// untranslated. A translation with a problem (see
    /// [`crate::check::check_translation`]) is refused and nothing is
    /// written. Saving a translation clears `fuzzy`.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::Invalid`] with the problems, or an error when
    /// the string is not an entry, its file is broken or does not match the
    /// game, or it cannot be written.
    pub fn set_translation(
        &self,
        sheet_name: &str,
        row: u32,
        subrow: u16,
        column: u32,
        text: &str,
    ) -> Result<Option<Translation>, EditError> {
        let knowledge = self.knowledge();
        let target = self.settings().target_language;
        self.edit(sheet_name, row, subrow, column, |entry| {
            if !text.is_empty() {
                let verdict = check_translation(
                    &knowledge,
                    &target,
                    &entry.source,
                    text,
                    &entry.extracted,
                    &entry.term_exceptions,
                );
                if !verdict.problems.is_empty() {
                    return Err(EditError::Invalid(
                        verdict
                            .issues
                            .into_iter()
                            .filter(crate::check::Issue::is_problem)
                            .collect(),
                    ));
                }
            }
            text.clone_into(&mut entry.translation);
            entry.fuzzy = false;
            entry.previous = None;
            Ok(())
        })
    }

    /// Adds or removes a term exception of a string: the glossary term does
    /// not apply to it, so the checks neither ask for its translation nor
    /// forbid its variants there, and machine translation is told so.
    ///
    /// # Errors
    ///
    /// Returns an error when the string is not an entry, or its file is
    /// broken, does not match the game, or cannot be written.
    pub fn set_term_exception(
        &self,
        sheet_name: &str,
        row: u32,
        subrow: u16,
        column: u32,
        term: &str,
        add: bool,
    ) -> Result<Option<Translation>, EditError> {
        self.edit(sheet_name, row, subrow, column, |entry| {
            set_term_exception(entry, term, add);
            Ok(())
        })
    }

    /// What the checks find in the saved translation of a string, problems
    /// then advice, and its term exceptions. An untranslated string has no
    /// findings.
    ///
    /// # Errors
    ///
    /// Returns an error when the string is not an entry, or its file cannot
    /// be read or is broken.
    pub fn findings(
        &self,
        sheet_name: &str,
        row: u32,
        subrow: u16,
        column: u32,
    ) -> Result<(Vec<crate::check::Issue>, Vec<String>), EditError> {
        let (path, identity, _) = self.locate(sheet_name, row, subrow, column)?;
        let full = self.root.join(PO_DIR).join(&path);
        let text = std::fs::read_to_string(&full).map_err(|source| ProjectError::Io {
            path: full.clone(),
            source,
        })?;
        let (file, problems) = PoFile::parse(&text);
        if let Some(problem) = problems.first() {
            return Err(EditError::Broken {
                path,
                line: problem.line,
                message: problem.message.clone(),
            });
        }
        let Some(entry) = file.entries.iter().find(|entry| entry.context == identity) else {
            return Ok((Vec::new(), Vec::new()));
        };
        let issues = if entry.translation.is_empty() {
            Vec::new()
        } else {
            check_translation(
                &self.knowledge(),
                &self.settings().target_language,
                &entry.source,
                &entry.translation,
                &entry.extracted,
                &entry.term_exceptions,
            )
            .issues
        };
        Ok((issues, entry.term_exceptions.clone()))
    }

    /// Sets or clears the translator's note of a string.
    ///
    /// # Errors
    ///
    /// Returns an error when the string is not an entry, or its file is
    /// broken, does not match the game, or cannot be written.
    pub fn set_note(
        &self,
        sheet_name: &str,
        row: u32,
        subrow: u16,
        column: u32,
        note: Option<&str>,
    ) -> Result<Option<Translation>, EditError> {
        let lines: Vec<String> = note
            .map(str::trim)
            .filter(|note| !note.is_empty())
            .map(|note| note.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        self.edit(sheet_name, row, subrow, column, |entry| {
            entry.notes = lines;
            Ok(())
        })
    }

    /// Writes translations of entries of one file, by `msgctxt`, in one
    /// write. Only an entry that is still untranslated (or still fuzzy, with
    /// `replace_fuzzy`) takes its translation, so work saved meanwhile is
    /// kept; the caller checked the translations. Returns the `msgctxt` of
    /// every entry written.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, breaks the format, or
    /// cannot be written.
    pub fn fill(
        &self,
        path: &str,
        translations: &[(String, String)],
        replace_fuzzy: bool,
    ) -> Result<Vec<String>, EditError> {
        let _writing = lock(&self.writing);
        let full = self.root.join(PO_DIR).join(path);
        let text = std::fs::read_to_string(&full).map_err(|source| ProjectError::Io {
            path: full.clone(),
            source,
        })?;
        let (mut file, problems) = PoFile::parse(&text);
        if let Some(problem) = problems.first() {
            return Err(EditError::Broken {
                path: path.to_owned(),
                line: problem.line,
                message: problem.message.clone(),
            });
        }
        let wanted: HashMap<&str, &str> = translations
            .iter()
            .filter(|(_, text)| !text.is_empty())
            .map(|(context, text)| (context.as_str(), text.as_str()))
            .collect();
        let mut written = Vec::new();
        for entry in &mut file.entries {
            let Some(text) = wanted.get(entry.context.as_str()) else {
                continue;
            };
            let open = if entry.fuzzy {
                replace_fuzzy
            } else {
                entry.translation.is_empty()
            };
            if open {
                (*text).clone_into(&mut entry.translation);
                entry.fuzzy = false;
                entry.previous = None;
                written.push(entry.context.clone());
            }
        }
        if !written.is_empty() {
            write_atomically(&full, &file.write())?;
            self.touch();
        }
        let now = stamp(&full);
        lock(&self.files).insert(path.to_owned(), (now, Arc::new(FileState::of(&file))));
        if let Some(seen) = lock(&self.viewed).get_mut(path) {
            // The editor shows this file: tell it, as for a change by Git.
            if written.is_empty() {
                *seen = now;
            }
        }
        Ok(written)
    }

    /// Applies edits of many entries, writing each file once. An edit is
    /// made only while its entry still has the expected translation and
    /// fuzzy mark; a replacement with a problem (see
    /// [`crate::check::check_translation`]) is not written. Skipped edits
    /// are reported, never fatal.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be read or written.
    pub fn apply_edits(&self, edits: &[EntryEdit]) -> Result<EditsApplied, EditError> {
        let knowledge = self.knowledge();
        let target = self.settings().target_language;
        let _writing = lock(&self.writing);
        let mut by_file: std::collections::BTreeMap<&str, Vec<&EntryEdit>> =
            std::collections::BTreeMap::new();
        for edit in edits {
            by_file.entry(edit.path.as_str()).or_default().push(edit);
        }
        let mut applied = EditsApplied::default();
        for (path, edits) in by_file {
            let skip_all = |applied: &mut EditsApplied, reason: SkipReason| {
                applied.skipped.extend(edits.iter().map(|edit| EditSkipped {
                    path: edit.path.clone(),
                    context: edit.context.clone(),
                    reason: reason.clone(),
                }));
            };
            let full = self.root.join(PO_DIR).join(path);
            let text = match std::fs::read_to_string(&full) {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    skip_all(&mut applied, SkipReason::Missing);
                    continue;
                }
                Err(source) => return Err(ProjectError::Io { path: full, source }.into()),
            };
            let (mut file, problems) = PoFile::parse(&text);
            if let Some(problem) = problems.first() {
                skip_all(
                    &mut applied,
                    SkipReason::Broken(format!("line {}: {}", problem.line, problem.message)),
                );
                continue;
            }
            let mut written = 0;
            for edit in edits {
                let Some(entry) = file
                    .entries
                    .iter_mut()
                    .find(|entry| entry.context == edit.context)
                else {
                    applied.skipped.push(EditSkipped {
                        path: edit.path.clone(),
                        context: edit.context.clone(),
                        reason: SkipReason::Missing,
                    });
                    continue;
                };
                match apply_edit(entry, edit, &knowledge, &target) {
                    Ok(Some(done)) => {
                        written += 1;
                        applied.done.push(done);
                    }
                    Ok(None) => {}
                    Err(reason) => applied.skipped.push(EditSkipped {
                        path: edit.path.clone(),
                        context: edit.context.clone(),
                        reason,
                    }),
                }
            }
            if written > 0 {
                write_atomically(&full, &file.write())?;
                self.touch();
            }
            // The editor learns of the change through `changed_sheets`, as
            // for a change by Git: its stamp of the file stays the old one.
            let now = stamp(&full);
            lock(&self.files).insert(path.to_owned(), (now, Arc::new(FileState::of(&file))));
        }
        Ok(applied)
    }

    /// How much of each sheet with entries is translated, by sheet name.
    /// Files are read again only when they changed, so the first call reads
    /// every file and later ones only the changed files.
    ///
    /// # Errors
    ///
    /// Returns an error when `po/` cannot be listed or a file cannot be read.
    pub fn progress(&self) -> Result<Vec<SheetProgress>, EditError> {
        let mut by_sheet: std::collections::BTreeMap<String, SheetProgress> =
            std::collections::BTreeMap::new();
        for path in list(&self.root)? {
            let state = self.file_state(&path)?;
            let Some(sheet) = &state.sheet else {
                continue;
            };
            let progress = by_sheet
                .entry(sheet.clone())
                .or_insert_with(|| SheetProgress {
                    sheet: sheet.clone(),
                    ..SheetProgress::default()
                });
            progress.entries += state.entries;
            progress.translated += state.translated;
            progress.fuzzy += state.fuzzy;
        }
        Ok(by_sheet.into_values().collect())
    }
}

/// Applies one edit to its entry: the edit done, `None` when it changed
/// nothing, or why it was not made.
fn apply_edit(
    entry: &mut crate::po::Entry,
    edit: &EntryEdit,
    knowledge: &Knowledge,
    target: &str,
) -> Result<Option<EditDone>, SkipReason> {
    if entry.translation != edit.expected_text || entry.fuzzy != edit.expected_fuzzy {
        return Err(SkipReason::Changed);
    }
    let before = EntryState::of(entry);
    match &edit.kind {
        EditKind::Replace(new) => {
            if !new.is_empty() {
                let verdict = check_translation(
                    knowledge,
                    target,
                    &entry.source,
                    new,
                    &entry.extracted,
                    &entry.term_exceptions,
                );
                if !verdict.problems.is_empty() {
                    return Err(SkipReason::Invalid(
                        verdict
                            .issues
                            .into_iter()
                            .filter(crate::check::Issue::is_problem)
                            .collect(),
                    ));
                }
            }
            new.clone_into(&mut entry.translation);
        }
        EditKind::Clear => {
            entry.translation.clear();
            entry.fuzzy = false;
            entry.previous = None;
        }
        EditKind::Restore(state) => {
            state.text.clone_into(&mut entry.translation);
            entry.fuzzy = state.fuzzy;
            entry.previous.clone_from(&state.previous);
            entry.term_exceptions.clone_from(&state.term_exceptions);
        }
        EditKind::TermException { term, add } => set_term_exception(entry, term, *add),
    }
    let after = EntryState::of(entry);
    Ok((after != before).then(|| EditDone {
        path: edit.path.clone(),
        context: edit.context.clone(),
        before,
        after,
    }))
}

/// Adds or removes the exception of `term`, compared ignoring case. A term
/// that cannot be written as a flag (see [`crate::po::can_be_exception`]) is
/// not added.
pub fn set_term_exception(entry: &mut crate::po::Entry, term: &str, add: bool) {
    let term = term.trim();
    let lower = term.to_lowercase();
    entry
        .term_exceptions
        .retain(|known| known.to_lowercase() != lower);
    if add && crate::po::can_be_exception(term) {
        entry.term_exceptions.push(term.to_owned());
    }
}

/// Writes a file through a temporary file and a rename, so a reader never
/// sees half of it.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_atomically(path: &Path, text: &str) -> Result<(), ProjectError> {
    let io = |source| ProjectError::Io {
        path: path.to_owned(),
        source,
    };
    let partial = path.with_extension("po.partial");
    std::fs::write(&partial, text).map_err(io)?;
    std::fs::rename(&partial, path).map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        io(error)
    })
}

/// Creates a project from the installed game in `root` and opens it; see
/// [`crate::project::create`].
///
/// # Errors
///
/// Returns an error when `root` already is a project, or the game or a file
/// cannot be read or written.
pub fn create(
    root: &Path,
    source: Arc<GameSource>,
    target_language: &str,
    threads: usize,
) -> Result<Session, ProjectError> {
    crate::project::create(root, &source, target_language, threads)?;
    let settings = read_settings(root)?;
    Ok(Session::new(root, settings, source))
}
