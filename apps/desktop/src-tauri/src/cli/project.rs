//! Opening a project without the desktop, and the strings of its sheets as
//! the command shows them.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use aeria_core::{ReviewState, SourceBinding};
use aeria_knowledge::{Knowledge, KnowledgeFile};
use aeria_search::{SimilarSearch, SimilarSource, SourceIndex, Tokenizer};
use aeria_source::{GameSource, LineRole, SheetLookup, SourceSheet};
use aeria_workspace::{ProjectSession, ProjectSessionError, WorkspaceStore};
use serde::Serialize;

use crate::games::{SETTINGS_FILE, resolve_game_path_with};
use crate::paths::{standalone_cache_dir, standalone_data_dir};
use crate::source::{load_catalog_with_cache, open_game_at};
use crate::sync::{Ledger, ProjectFiles};

/// The directory that marks a project root.
const WORKSPACE_DIRECTORY: &str = ".aeria";

/// Phase timings on standard error when `AERIA_TRACE` is set.
pub(crate) struct Trace(Option<std::time::Instant>);

impl Trace {
    pub(crate) fn start() -> Self {
        Self(std::env::var_os("AERIA_TRACE").map(|_| std::time::Instant::now()))
    }

    pub(crate) fn mark(&self, what: &str) {
        if let Some(start) = self.0 {
            eprintln!("aeria trace: {what} at {} ms", start.elapsed().as_millis());
        }
    }
}

/// Where the command finds Aeria's data, caches, and the game.
#[derive(Clone, Debug, Default)]
pub(crate) struct Env {
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    /// The game installation; `None` for the one chosen in Aeria's
    /// settings.
    pub game_path: Option<String>,
}

impl Env {
    /// The folders the desktop uses on this computer.
    pub(crate) fn standalone() -> Self {
        Self {
            data_dir: standalone_data_dir(),
            cache_dir: standalone_cache_dir(),
            game_path: None,
        }
    }

    /// The shared files of the project at or above `start`, without
    /// opening it.
    pub(crate) fn project_files(&self, start: &Path) -> Result<Option<ProjectFiles>, String> {
        let root = find_root(start)?;
        Ok(self
            .data_dir
            .as_ref()
            .map(|data_dir| ProjectFiles::new(data_dir, &root)))
    }

    fn game_path(&self) -> Result<String, String> {
        if let Some(path) = &self.game_path {
            return Ok(path.clone());
        }
        let settings = self.data_dir.as_ref().map_or_else(
            || PathBuf::from(SETTINGS_FILE),
            |dir| dir.join(SETTINGS_FILE),
        );
        resolve_game_path_with(&settings).map_err(|error| error.message)
    }
}

/// Similar sources looked up per source text.
const SIMILAR_LOOKUP: usize = 60;

/// The size and time of each knowledge file, to tell when to read them again.
type KnowledgeStamp = Vec<Option<(u64, SystemTime)>>;

/// An open project, and what the command learned about it: in a server
/// process these caches live as long as the project is open.
pub(crate) struct Project {
    pub session: ProjectSession,
    /// The shared files of the project; `None` when the data folder is
    /// unknown.
    pub files: Option<ProjectFiles>,
    pub data_dir: Option<PathBuf>,
    knowledge: Mutex<Option<(KnowledgeStamp, Arc<Knowledge>)>>,
    index: OnceLock<Option<SourceIndex>>,
    /// How many indexed strings contain each word.
    word_counts: OnceLock<Option<Arc<HashMap<String, i64>>>>,
    /// Idle similar-string searchers, each with its own connection.
    searchers: Mutex<Vec<SimilarSearch>>,
    /// Similar sources by source text; the game's text does not change while
    /// the project is open.
    similar: Mutex<HashMap<String, Arc<Vec<SimilarSource>>>>,
    /// The strings of sheets, in order; also game data.
    sheets: Mutex<HashMap<String, Arc<Vec<SheetLine>>>>,
}

/// The project root: `start` or the nearest parent with an `.aeria`
/// directory.
pub(crate) fn find_root(start: &Path) -> Result<PathBuf, String> {
    let start =
        std::fs::canonicalize(start).map_err(|error| format!("{}: {error}", start.display()))?;
    start
        .ancestors()
        .find(|path| path.join(WORKSPACE_DIRECTORY).is_dir())
        .map(Path::to_owned)
        .ok_or_else(|| {
            format!(
                "{} is not inside an Aeria project (no {WORKSPACE_DIRECTORY} directory); run the command in the project or pass --project",
                start.display()
            )
        })
}

impl Project {
    /// Opens the project at or above `start` against the game installation
    /// chosen in Aeria's settings.
    pub(crate) fn open(start: &Path, env: &Env) -> Result<Self, String> {
        let trace = Trace::start();
        let root = find_root(start)?;
        let metadata = WorkspaceStore::new(&root)
            .read_metadata()
            .map_err(|error| error.to_string())?;
        let data_dir = env.data_dir.clone();
        let game_path = env.game_path()?;
        let source: Arc<GameSource> =
            open_game_at(&game_path, metadata.source_language()).map_err(|error| error.message)?;
        trace.mark("game opened");
        if let Some(cache) = &env.cache_dir {
            load_catalog_with_cache(cache, &source).map_err(|error| error.message)?;
        }
        trace.mark("catalog loaded");
        let session = ProjectSession::open(&root, source).map_err(|error| match error {
            ProjectSessionError::SourceUpdateRequired { .. } => format!(
                "the project was made for another game version; open it in Aeria and update it first ({error})"
            ),
            other => other.to_string(),
        })?;
        trace.mark("workspace opened");
        let files = data_dir.as_ref().map(|dir| ProjectFiles::new(dir, &root));
        Ok(Self {
            session,
            files,
            data_dir,
            knowledge: Mutex::new(None),
            index: OnceLock::new(),
            word_counts: OnceLock::new(),
            searchers: Mutex::new(Vec::new()),
            similar: Mutex::new(HashMap::new()),
            sheets: Mutex::new(HashMap::new()),
        })
    }

    /// The project knowledge, read again only when a file changed.
    pub(crate) fn knowledge(&self) -> Arc<Knowledge> {
        let stamp: KnowledgeStamp = KnowledgeFile::ALL
            .iter()
            .map(|file| {
                std::fs::metadata(file.path(self.root()))
                    .ok()
                    .and_then(|metadata| Some((metadata.len(), metadata.modified().ok()?)))
            })
            .collect();
        let mut cached = self
            .knowledge
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((seen, knowledge)) = cached.as_ref()
            && *seen == stamp
        {
            return Arc::clone(knowledge);
        }
        let knowledge = Arc::new(Knowledge::load(self.root()));
        *cached = Some((stamp, Arc::clone(&knowledge)));
        knowledge
    }

    fn search_key(&self) -> String {
        let source = self.session.source();
        format!("{}/{}", source.language(), source.version())
    }

    fn search_index_path(&self) -> Option<PathBuf> {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.search_key().as_bytes());
        let name = digest[..16]
            .iter()
            .fold(String::with_capacity(32), |mut name, byte| {
                let _ = write!(name, "{byte:02x}");
                name
            });
        self.data_dir
            .as_ref()
            .map(|dir| dir.join("search").join(format!("{name}.sqlite3")))
    }

    /// The search index of the project's game data, when it was built.
    pub(crate) fn search_index(&self) -> Option<&SourceIndex> {
        self.index
            .get_or_init(|| {
                SourceIndex::open(self.search_index_path()?, &self.search_key())
                    .ok()
                    .flatten()
            })
            .as_ref()
    }

    /// The search index, built first when missing: building reads every
    /// sheet of the game once. `notice` is told before a build.
    pub(crate) fn search_index_or_build(
        &self,
        notice: &mut dyn FnMut(&str),
    ) -> Result<SourceIndex, String> {
        if let Some(index) = self.search_index() {
            return Ok(index.clone());
        }
        let path = self
            .search_index_path()
            .ok_or("Aeria's data folder is unknown, so there is no search index")?;
        notice("building the search index of this game version once; this takes a few minutes");
        let source = self.session.source_handle();
        SourceIndex::build(
            path,
            &self.search_key(),
            Tokenizer::for_language(source.language().code()),
            &source,
            &|| true,
        )
        .map_err(|error| format!("the search index could not be built: {error}"))
    }

    fn word_counts(&self) -> Option<Arc<HashMap<String, i64>>> {
        self.word_counts
            .get_or_init(|| {
                self.search_index()
                    .and_then(|index| index.word_counts().ok())
                    .map(Arc::new)
            })
            .clone()
    }

    /// Loads what the first commands would otherwise wait for: the search
    /// index and its word counts, and the knowledge.
    pub(crate) fn warm(&self) {
        let _ = self.word_counts();
        let _ = self.knowledge();
    }

    /// Strings of the game similar to a source text, most similar first;
    /// empty without a search index.
    pub(crate) fn similar_sources(&self, source: &str) -> Arc<Vec<SimilarSource>> {
        if let Some(found) = self
            .similar
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(source)
        {
            return Arc::clone(found);
        }
        let Some(index) = self.search_index() else {
            return Arc::new(Vec::new());
        };
        let searcher = self
            .searchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .map_or_else(|| index.similar_search_with(self.word_counts()), Ok);
        let Ok(searcher) = searcher else {
            return Arc::new(Vec::new());
        };
        let found = Arc::new(
            searcher
                .similar(source, None, SIMILAR_LOOKUP)
                .unwrap_or_default(),
        );
        self.searchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(searcher);
        self.similar
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(source.to_owned(), Arc::clone(&found));
        found
    }

    pub(crate) fn root(&self) -> &Path {
        self.session.repository_root()
    }

    /// The project root as people write it, without the `\\?\` prefix of
    /// a canonical Windows path.
    pub(crate) fn root_display(&self) -> String {
        let root = self.root().to_string_lossy();
        root.strip_prefix(r"\\?\").unwrap_or(&root).to_owned()
    }

    /// The target language code; `None` while the project has none.
    pub(crate) fn target_language(&self) -> Option<String> {
        let tag = self
            .session
            .workspace()
            .metadata()
            .target_language()
            .to_owned();
        (tag != "und").then_some(tag)
    }

    pub(crate) fn source_language(&self) -> String {
        self.session.source().language().code().to_owned()
    }

    pub(crate) fn ledger(&self) -> Option<Ledger> {
        self.files.as_ref().and_then(|files| files.ledger().ok())
    }

    /// A sheet of the game; an error names a sheet that does not exist.
    pub(crate) fn sheet(&self, name: &str) -> Result<Arc<SourceSheet>, String> {
        match self
            .session
            .source()
            .sheet(name)
            .map_err(|error| error.to_string())?
        {
            SheetLookup::Present(sheet) => Ok(sheet),
            SheetLookup::Missing => Err(format!(
                "there is no sheet {name:?}; `aeria overview <pattern>` lists sheets"
            )),
            SheetLookup::Unavailable(reason) => {
                Err(format!("the sheet {name} cannot be read: {reason:?}"))
            }
        }
    }
}

/// `sheet:row:subrow:column`, the address of one string.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct Address {
    pub sheet: String,
    pub row: u32,
    pub subrow: u16,
    pub column: u32,
}

impl Address {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim().trim_start_matches('@');
        let mut parts = text.rsplitn(4, ':');
        let (Some(column), Some(subrow), Some(row), Some(sheet)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(format!(
                "{text:?} is not an address; addresses are sheet:row:subrow:column, as `aeria read` prints them"
            ));
        };
        let number = |part: &str| format!("{text:?}: {part:?} is not a number");
        Ok(Self {
            sheet: sheet.to_owned(),
            row: row.parse().map_err(|_| number(row))?,
            subrow: subrow.parse().map_err(|_| number(subrow))?,
            column: column.parse().map_err(|_| number(column))?,
        })
    }

    pub(crate) fn binding(&self) -> SourceBinding {
        SourceBinding::new(self.sheet.clone(), self.row, self.subrow, self.column)
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}:{}:{}:{}",
            self.sheet, self.row, self.subrow, self.column
        )
    }
}

/// Who wrote a translation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Author {
    /// An agent, and nobody changed it since.
    Agent,
    /// A person, or a translation written before agents were recorded.
    Person,
}

/// A string's current translation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Current {
    pub target: String,
    #[serde(serialize_with = "serialize_review")]
    pub review: ReviewState,
    pub author: Author,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Current {
    /// Whether an agent may replace it: it is an agent's and not reviewed.
    pub(crate) fn replaceable(&self) -> bool {
        self.author == Author::Agent && self.review != ReviewState::Reviewed
    }
}

/// The current translation of a string, with who wrote it.
/// Whether the string has a translation; cheaper than [`current`], which
/// also asks the ledger who wrote it.
pub(crate) fn translated(session: &ProjectSession, address: &Address) -> bool {
    session
        .workspace()
        .unit_by_source_binding(&address.binding())
        .is_some()
}

pub(crate) fn current(
    session: &ProjectSession,
    ledger: Option<&Ledger>,
    address: &Address,
) -> Option<Current> {
    let unit = session
        .workspace()
        .unit_by_source_binding(&address.binding())?;
    let target = unit.target_macro().to_owned();
    let agents = ledger
        .and_then(|ledger| ledger.agent_target(&address.to_string()).ok().flatten())
        .is_some_and(|written| written == target);
    Some(Current {
        review: unit.review_state(),
        author: if agents {
            Author::Agent
        } else {
            Author::Person
        },
        note: unit.translator_note().map(str::to_owned),
        target,
    })
}

/// What kind of line a string is.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "speaker")]
pub(crate) enum LineKind {
    Journal,
    Objective,
    Speech(String),
    /// A dialogue row with no known role.
    Other,
    /// A string of a sheet that is not dialogue.
    Text,
}

impl LineKind {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Journal => "journal".to_owned(),
            Self::Objective => "objective".to_owned(),
            Self::Speech(speaker) => speaker.clone(),
            Self::Other => "other".to_owned(),
            Self::Text => "text".to_owned(),
        }
    }
}

/// One translatable string of a sheet.
#[derive(Clone, Debug)]
pub(crate) struct SheetLine {
    pub address: Address,
    pub kind: LineKind,
    pub source: String,
    /// The row's other String cells, as `(column, text)`.
    pub context: Vec<(u32, String)>,
}

/// The translatable strings of a sheet in order: a quest or cutscene in the
/// order of its dialogue, any other sheet in row order.
pub(crate) fn sheet_lines(project: &Project, sheet: &str) -> Result<Arc<Vec<SheetLine>>, String> {
    if let Some(lines) = project
        .sheets
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(sheet)
    {
        return Ok(Arc::clone(lines));
    }
    let lines = Arc::new(read_sheet_lines(project, sheet)?);
    project
        .sheets
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(sheet.to_owned(), Arc::clone(&lines));
    Ok(lines)
}

fn read_sheet_lines(project: &Project, sheet: &str) -> Result<Vec<SheetLine>, String> {
    let game = project.sheet(sheet)?;
    let source = project.session.source();
    if let Some(dialogue) = source.dialogue(sheet).map_err(|error| error.to_string())? {
        return Ok(dialogue
            .lines
            .into_iter()
            .filter(|line| {
                game.cell(line.row_id, line.subrow_id, line.column)
                    .is_some_and(|cell| cell.translatable)
            })
            .map(|line| SheetLine {
                address: Address {
                    sheet: sheet.to_owned(),
                    row: line.row_id,
                    subrow: line.subrow_id,
                    column: line.column,
                },
                kind: match line.role {
                    LineRole::Journal => LineKind::Journal,
                    LineRole::Objective => LineKind::Objective,
                    LineRole::Speech { speaker } => LineKind::Speech(speaker),
                    LineRole::Other => LineKind::Other,
                },
                source: line.text,
                context: Vec::new(),
            })
            .collect());
    }
    let mut lines = Vec::new();
    for row in game.rows() {
        let cells: Vec<_> = game.cells(row).collect();
        for cell in cells.iter().filter(|cell| cell.translatable) {
            let text = cell.text();
            if text.trim().is_empty() {
                continue;
            }
            lines.push(SheetLine {
                address: Address {
                    sheet: sheet.to_owned(),
                    row: row.row_id,
                    subrow: row.subrow_id,
                    column: cell.column,
                },
                kind: LineKind::Text,
                source: text,
                context: cells
                    .iter()
                    .filter(|other| other.column != cell.column)
                    .map(|other| (other.column, other.text()))
                    .filter(|(_, text)| !text.trim().is_empty())
                    .collect(),
            });
        }
    }
    Ok(lines)
}

/// The string at an address, if it is translatable.
pub(crate) fn source_of(project: &Project, address: &Address) -> Result<String, String> {
    project
        .session
        .source_macro(&address.binding())
        .map_err(|_| format!("{address} is not a translatable string"))
}

/// The line in the game's other client languages, as `(code, text)`.
pub(crate) fn other_languages(project: &Project, address: &Address) -> Vec<(String, String)> {
    // Other languages are evidence; one that cannot be read is left out.
    project
        .session
        .source()
        .cell_in_other_languages(&address.sheet, address.row, address.subrow, address.column)
        .map(|texts| {
            texts
                .into_iter()
                .filter_map(|(language, text)| {
                    text.filter(|text| !text.trim().is_empty())
                        .map(|text| (language.code().to_owned(), text))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Serializes a review state as its word.
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
pub(crate) fn serialize_review<S: serde::Serializer>(
    review: &ReviewState,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(review_word(*review))
}

/// The review state as a word.
pub(crate) const fn review_word(review: ReviewState) -> &'static str {
    match review {
        ReviewState::Draft => "draft",
        ReviewState::NeedsReview => "needs review",
        ReviewState::Reviewed => "reviewed",
    }
}

/// Matches a sheet name against a pattern: `*` is any run of characters,
/// `?` one character; a pattern without them matches names that contain
/// it. Case is ignored.
pub(crate) fn matches_pattern(name: &str, pattern: &str) -> bool {
    let name = name.to_lowercase();
    let pattern = pattern.to_lowercase();
    if !pattern.contains(['*', '?']) {
        return name.contains(&pattern);
    }
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut n, mut p) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            n += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = n;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|character| *character == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_patterns_parse() {
        let address = Address::parse("@quest/000/ClsGla001_00001:12:0:1").expect("address");
        assert_eq!(address.sheet, "quest/000/ClsGla001_00001");
        assert_eq!((address.row, address.subrow, address.column), (12, 0, 1));
        assert_eq!(address.to_string(), "quest/000/ClsGla001_00001:12:0:1");
        assert!(Address::parse("Addon:1:0").is_err());
        assert!(Address::parse("Addon:x:0:1").is_err());

        assert!(matches_pattern("quest/000/ClsGla001_00001", "quest/*"));
        assert!(matches_pattern("quest/000/ClsGla001_00001", "*gla*"));
        assert!(matches_pattern("Addon", "addon"));
        assert!(matches_pattern("PlaceName", "place"));
        assert!(!matches_pattern("Addon", "quest/*"));
        assert!(matches_pattern("Item", "Ite?"));
        assert!(!matches_pattern("Items", "Ite?"));
    }
}
