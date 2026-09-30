//! Opening a project without the desktop: its settings, the installed game,
//! and the project knowledge.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use aeria_knowledge::{Knowledge, KnowledgeFile};
use aeria_po::{SETTINGS_FILE, Settings};
use aeria_source::GameSource;

use crate::games::{SETTINGS_FILE as GAME_SETTINGS_FILE, resolve_game_path_with};
use crate::paths::standalone_data_dir;
use crate::source::open_game_at;
use crate::sync::ProjectFiles;

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

/// Where the command finds Aeria's data and the game.
#[derive(Clone, Debug, Default)]
pub(crate) struct Env {
    pub data_dir: Option<PathBuf>,
    /// The game installation; `None` for the one chosen in Aeria's
    /// settings.
    pub game_path: Option<String>,
}

impl Env {
    /// The folders the desktop uses on this computer.
    pub(crate) fn standalone() -> Self {
        Self {
            data_dir: standalone_data_dir(),
            game_path: None,
        }
    }

    /// The files in application data that belong to the project at or above
    /// `start` (the server's), without opening it.
    pub(crate) fn project_files(&self, start: &Path) -> Result<Option<ProjectFiles>, String> {
        let root = find_root(start)?;
        Ok(self
            .data_dir
            .as_ref()
            .map(|data_dir| ProjectFiles::new(data_dir, &root)))
    }

    /// The installed game: the one chosen in Aeria's settings, or the one
    /// found on this computer.
    pub(crate) fn game_path(&self) -> Result<String, String> {
        if let Some(path) = &self.game_path {
            return Ok(path.clone());
        }
        let settings = self.data_dir.as_ref().map_or_else(
            || PathBuf::from(GAME_SETTINGS_FILE),
            |dir| dir.join(GAME_SETTINGS_FILE),
        );
        resolve_game_path_with(&settings).map_err(|error| error.message)
    }

    /// Opens the installed game in a source language.
    pub(crate) fn open_game(&self, language: &str) -> Result<Arc<GameSource>, String> {
        open_game_at(&self.game_path()?, language).map_err(|error| error.message)
    }
}

/// The project root: `start` or the nearest parent with `aeria.json`.
pub(crate) fn find_root(start: &Path) -> Result<PathBuf, String> {
    let start =
        std::fs::canonicalize(start).map_err(|error| format!("{}: {error}", start.display()))?;
    start
        .ancestors()
        .find(|path| path.join(SETTINGS_FILE).is_file())
        .map(Path::to_owned)
        .ok_or_else(|| {
            format!(
                "{} is not inside an Aeria project (no {SETTINGS_FILE}); run the command in the project, pass --project, or make one with `aeria init --language <tag>`",
                start.display()
            )
        })
}

/// The size and time of each knowledge file, to tell when to read them again.
type KnowledgeStamp = Vec<Option<(u64, SystemTime)>>;

/// An open project. In a server process it stays open between commands.
pub(crate) struct Project {
    pub root: PathBuf,
    pub settings: Settings,
    pub source: Arc<GameSource>,
    knowledge: Mutex<Option<(KnowledgeStamp, Arc<Knowledge>)>>,
}

impl Project {
    /// Opens the project at or above `start` against the installed game.
    pub(crate) fn open(start: &Path, env: &Env) -> Result<Self, String> {
        let trace = Trace::start();
        let root = find_root(start)?;
        let settings = aeria_po::read_settings(&root).map_err(|error| error.to_string())?;
        let source = env.open_game(&settings.source_language)?;
        trace.mark("game opened");
        Ok(Self {
            root,
            settings,
            source,
            knowledge: Mutex::new(None),
        })
    }

    /// Whether the game on disk is still the version this project opened: a
    /// game update replaces it while a server runs.
    pub(crate) fn game_current(&self) -> bool {
        self.source
            .current_version()
            .is_ok_and(|version| version == *self.source.version())
    }

    /// The project knowledge, read again only when a file changed.
    pub(crate) fn knowledge(&self) -> Arc<Knowledge> {
        let stamp: KnowledgeStamp = KnowledgeFile::ALL
            .iter()
            .map(|file| {
                std::fs::metadata(file.path(&self.root))
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
        let knowledge = Arc::new(Knowledge::load(&self.root));
        *cached = Some((stamp, Arc::clone(&knowledge)));
        knowledge
    }

    /// The root as the user reads it.
    pub(crate) fn root_display(&self) -> String {
        let text = self.root.to_string_lossy();
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }

    /// Threads for work over every sheet.
    pub(crate) fn threads() -> usize {
        std::thread::available_parallelism().map_or(4, |count| count.get().min(8))
    }
}
