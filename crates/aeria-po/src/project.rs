//! A project on disk: `aeria.json` and the files of `po/`, made from the
//! installed game, read, written, and brought to a new game version.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use aeria_source::GameSource;
use serde::{Deserialize, Serialize};

use crate::generate::{GenerateError, Languages, sheet_files};
use crate::identity::SheetPaths;
use crate::merge::merge_files;
use crate::po::{PoFile, Problem};

/// The project settings file at the root.
pub const SETTINGS_FILE: &str = "aeria.json";
/// The folder of the project's PO files.
pub const PO_DIR: &str = "po";
/// What agents read first, written by Aeria: `po/README.md`.
pub const README_PATH: &str = "po/README.md";
/// The value of `format` in [`SETTINGS_FILE`].
pub const FORMAT: &str = "aeria-po/1";
/// The header field with the game version a file was made for.
pub const GAME_VERSION_FIELD: &str = "X-Game-Version";

/// Problems of files, each with the file's path relative to `po/`.
pub type FileProblems = Vec<(String, Problem)>;

/// `aeria.json`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub format: String,
    /// The source language code, such as `en`.
    pub source_language: String,
    /// The target language tag, such as `ru`.
    pub target_language: String,
}

/// Errors of a project on disk.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Settings { path: PathBuf, message: String },
    #[error("{0} is already an Aeria project")]
    Exists(PathBuf),
    #[error(
        "{path} is not an Aeria project: it has no {SETTINGS_FILE}; a project of an earlier format is not supported, start a new one"
    )]
    Missing { path: PathBuf },
    #[error(transparent)]
    Generate(#[from] GenerateError),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ProjectError + '_ {
    move |source| ProjectError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Reads `aeria.json`.
///
/// # Errors
///
/// Returns an error when the file is missing, cannot be read, or is not
/// settings of this format.
pub fn read_settings(root: &Path) -> Result<Settings, ProjectError> {
    let path = root.join(SETTINGS_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProjectError::Missing {
                path: root.to_owned(),
            });
        }
        Err(error) => return Err(io(&path)(error)),
    };
    let settings: Settings =
        serde_json::from_slice(&bytes).map_err(|error| ProjectError::Settings {
            path: path.clone(),
            message: error.to_string(),
        })?;
    if settings.format != FORMAT {
        return Err(ProjectError::Settings {
            path,
            message: format!(
                "format {:?} is not {FORMAT:?}; this version of Aeria reads only {FORMAT}",
                settings.format
            ),
        });
    }
    Ok(settings)
}

/// Writes `aeria.json` in canonical form.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_settings(root: &Path, settings: &Settings) -> Result<(), ProjectError> {
    let path = root.join(SETTINGS_FILE);
    let mut text =
        serde_json::to_string_pretty(settings).map_err(|error| ProjectError::Settings {
            path: path.clone(),
            message: error.to_string(),
        })?;
    text.push('\n');
    std::fs::write(&path, text).map_err(io(&path))
}

/// Every file of the game, made with empty translations, by path relative to
/// `po/`. Sheets are read on `threads` threads.
///
/// # Errors
///
/// Returns an error when a game file cannot be read.
pub fn make(
    source: &GameSource,
    languages: &Languages,
    threads: usize,
) -> Result<BTreeMap<String, PoFile>, GenerateError> {
    let names = source.sheet_names();
    let paths = SheetPaths::new(names.iter().map(String::as_str));
    let next = AtomicUsize::new(0);
    let files = Mutex::new(BTreeMap::new());
    let failure: Mutex<Option<GenerateError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(name) = names.get(index) else {
                        break;
                    };
                    match sheet_files(source, &paths, languages, name) {
                        Ok(made) => files
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend(made),
                        Err(error) => {
                            *failure
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = failure
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        return Err(error);
    }
    Ok(files
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner))
}

/// The `.po` files under `po/`, by path relative to it with `/`.
///
/// # Errors
///
/// Returns an error when a folder cannot be listed.
pub fn list(root: &Path) -> Result<Vec<String>, ProjectError> {
    let base = root.join(PO_DIR);
    let mut found = Vec::new();
    let mut folders = vec![base.clone()];
    while let Some(folder) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io(&folder)(error)),
        };
        for entry in entries {
            let path = entry.map_err(io(&folder))?.path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "po")
                && let Ok(relative) = path.strip_prefix(&base)
            {
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Reads files of `po/` by their paths relative to it. Each file's problems
/// come with its path.
///
/// # Errors
///
/// Returns an error when a file cannot be read.
pub fn read(
    root: &Path,
    paths: &[String],
) -> Result<(BTreeMap<String, PoFile>, FileProblems), ProjectError> {
    let base = root.join(PO_DIR);
    let mut files = BTreeMap::new();
    let mut problems = Vec::new();
    for path in paths {
        let full = base.join(path);
        let text = std::fs::read_to_string(&full).map_err(io(&full))?;
        let (file, found) = PoFile::parse(&text);
        problems.extend(found.into_iter().map(|problem| (path.clone(), problem)));
        files.insert(path.clone(), file);
    }
    Ok((files, problems))
}

/// Makes `po/` hold exactly `files`: writes each file whose text differs,
/// and removes the other `.po` files and the folders left empty. Returns how
/// many files were written or removed.
///
/// # Errors
///
/// Returns an error when a file cannot be written or removed.
pub fn write(root: &Path, files: &BTreeMap<String, PoFile>) -> Result<usize, ProjectError> {
    let base = root.join(PO_DIR);
    let mut changed = 0;
    for (path, file) in files {
        let full = base.join(path);
        let text = file.write();
        if std::fs::read(&full).ok().as_deref() == Some(text.as_bytes()) {
            continue;
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&full, text).map_err(io(&full))?;
        changed += 1;
    }
    // A system that ignores case lists a folder in whichever casing it was
    // made with.
    let wanted: std::collections::HashSet<String> =
        files.keys().map(|path| path.to_lowercase()).collect();
    for path in list(root)? {
        if wanted.contains(&path.to_lowercase()) {
            continue;
        }
        let full = base.join(&path);
        std::fs::remove_file(&full).map_err(io(&full))?;
        changed += 1;
        let mut parent = full.parent();
        while let Some(folder) = parent.filter(|folder| *folder != base) {
            if std::fs::remove_dir(folder).is_err() {
                break;
            }
            parent = folder.parent();
        }
    }
    Ok(changed)
}

/// Makes `root` a project: `aeria.json` and every file of the installed game
/// with empty translations. Returns how many files were written.
///
/// # Errors
///
/// Returns an error when `root` already is a project, or the game or a file
/// cannot be read or written.
pub fn create(
    root: &Path,
    source: &GameSource,
    target_language: &str,
    threads: usize,
) -> Result<usize, ProjectError> {
    if root.join(SETTINGS_FILE).exists() || root.join(PO_DIR).exists() {
        return Err(ProjectError::Exists(root.to_owned()));
    }
    std::fs::create_dir_all(root).map_err(io(root))?;
    let settings = Settings {
        format: FORMAT.to_owned(),
        source_language: source.language().code().to_owned(),
        target_language: target_language.to_owned(),
    };
    let files = make(
        source,
        &Languages {
            target: settings.target_language.clone(),
        },
        threads,
    )?;
    write_settings(root, &settings)?;
    write(root, &files)
}

/// What a game update did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Updated {
    /// Files written or removed.
    pub files: usize,
    /// Entries marked fuzzy by this update.
    pub fuzzy: usize,
    /// Translated entries that became obsolete in this update.
    pub obsolete: usize,
}

/// The game versions the files of a project were made for.
#[must_use]
pub fn versions(files: &BTreeMap<String, PoFile>) -> Vec<String> {
    let mut versions: Vec<String> = files
        .values()
        .filter_map(|file| file.field(GAME_VERSION_FIELD).map(str::to_owned))
        .collect();
    versions.sort();
    versions.dedup();
    versions
}

/// Brings every file to the installed game: see
/// [`crate::merge::merge_files`]. The caller makes sure the files hold no
/// changes that are not committed.
///
/// # Errors
///
/// Returns an error when the settings, the game, or a file cannot be read,
/// a file breaks the format, or a file cannot be written.
pub fn update(root: &Path, source: &GameSource, threads: usize) -> Result<Updated, ProjectError> {
    let settings = read_settings(root)?;
    let (previous, problems) = read(root, &list(root)?)?;
    if let Some((path, problem)) = problems.first() {
        return Err(ProjectError::Settings {
            path: root.join(PO_DIR).join(path),
            message: format!(
                "line {}: {}; fix the files before updating",
                problem.line, problem.message
            ),
        });
    }
    let fresh = make(
        source,
        &Languages {
            target: settings.target_language,
        },
        threads,
    )?;
    let version = source.version().to_string();
    let merged = merge_files(&previous, fresh, &version);
    let count = |files: &BTreeMap<String, PoFile>| {
        files.values().fold((0, 0), |(fuzzy, obsolete), file| {
            (
                fuzzy + file.entries.iter().filter(|entry| entry.fuzzy).count(),
                obsolete + file.obsolete.len(),
            )
        })
    };
    let (fuzzy_before, obsolete_before) = count(&previous);
    let (fuzzy_after, obsolete_after) = count(&merged);
    let files = write(root, &merged)?;
    Ok(Updated {
        files,
        fuzzy: fuzzy_after.saturating_sub(fuzzy_before),
        obsolete: obsolete_after.saturating_sub(obsolete_before),
    })
}
