//! One project written by several processes on this computer: the desktop
//! and any number of `aeria` commands run by agents.
//!
//! Per project, keyed by a hash of its canonical root, application data holds
//! `agents/<key>.lock`, `agents/<key>.stamp`, and `agents/<key>.sqlite3`:
//!
//! - Every workspace write takes an exclusive lock on the lock file for its
//!   whole run, so writes from different processes never interleave.
//! - A command that wrote translations replaces the stamp before it releases
//!   the lock. A process that keeps the workspace in memory compares the stamp
//!   with the one it last saw and reloads the workspace when it changed.
//! - The ledger records the translations agents wrote, with their text. A
//!   translation is an agent's while its text is still the one recorded; a
//!   person's edit changes the text, and the translation becomes theirs.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

const AGENTS_DIRECTORY: &str = "agents";
/// How long a ledger write waits for another process.
const LEDGER_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A hash of the canonical project root, so paths never appear in file
/// names and every process finds the same files for one project.
#[must_use]
pub(crate) fn project_key(root: &Path) -> String {
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    digest[..16]
        .iter()
        .fold(String::with_capacity(32), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        })
}

/// The shared files of one project.
#[derive(Clone, Debug)]
pub(crate) struct ProjectFiles {
    lock: PathBuf,
    stamp: PathBuf,
    ledger: PathBuf,
}

impl ProjectFiles {
    #[must_use]
    pub(crate) fn new(data_dir: &Path, root: &Path) -> Self {
        let directory = data_dir.join(AGENTS_DIRECTORY);
        let key = project_key(root);
        Self {
            lock: directory.join(format!("{key}.lock")),
            stamp: directory.join(format!("{key}.stamp")),
            ledger: directory.join(format!("{key}.sqlite3")),
        }
    }

    fn directory(&self) -> &Path {
        self.lock.parent().unwrap_or_else(|| Path::new("."))
    }

    /// The stamp of the last write by a command, if any.
    #[must_use]
    pub(crate) fn stamp(&self) -> Option<String> {
        fs::read_to_string(&self.stamp).ok()
    }

    /// Replaces the stamp. Call while holding the [`WriteLock`].
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the stamp cannot be written.
    pub(crate) fn touch_stamp(&self) -> std::io::Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let partial = self.stamp.with_extension("stamp.partial");
        fs::write(&partial, format!("{now}-{}", std::process::id()))?;
        fs::rename(&partial, &self.stamp)
    }

    /// Opens the ledger of agent translations, creating it when missing.
    ///
    /// # Errors
    ///
    /// Returns a description when the ledger cannot be opened.
    pub(crate) fn ledger(&self) -> Result<Ledger, String> {
        fs::create_dir_all(self.directory())
            .map_err(|error| format!("{}: {error}", self.directory().display()))?;
        let connection = Connection::open(&self.ledger)
            .map_err(|error| format!("{}: {error}", self.ledger.display()))?;
        connection
            .busy_timeout(LEDGER_BUSY_TIMEOUT)
            .and_then(|()| {
                connection.execute_batch(
                    "CREATE TABLE IF NOT EXISTS written (
                        location TEXT PRIMARY KEY,
                        target TEXT NOT NULL,
                        written_ms INTEGER NOT NULL
                    );",
                )
            })
            .map_err(|error| format!("{}: {error}", self.ledger.display()))?;
        Ok(Ledger { connection })
    }
}

/// An exclusive lock on a project's workspace writes, released when
/// dropped.
#[derive(Debug)]
pub(crate) struct WriteLock {
    _file: File,
}

impl WriteLock {
    /// Waits for and takes the lock.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the lock file cannot be opened or locked.
    pub(crate) fn acquire(files: &ProjectFiles) -> std::io::Result<Self> {
        fs::create_dir_all(files.directory())?;
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&files.lock)?;
        file.lock()?;
        Ok(Self { _file: file })
    }
}

/// The translations agents wrote in one project.
pub(crate) struct Ledger {
    connection: Connection,
}

impl Ledger {
    /// The text an agent last wrote at `location`, if any.
    ///
    /// # Errors
    ///
    /// Returns a description when the ledger cannot be read.
    pub(crate) fn agent_target(&self, location: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT target FROM written WHERE location = ?1",
                params![location],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Records translations agents wrote, as `(location, target)`.
    ///
    /// # Errors
    ///
    /// Returns a description when the ledger cannot be written.
    pub(crate) fn record(&mut self, written: &[(String, String)]) -> Result<(), String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
            });
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        {
            let mut upsert = transaction
                .prepare(
                    "INSERT INTO written (location, target, written_ms) VALUES (?1, ?2, ?3)
                     ON CONFLICT(location) DO UPDATE SET target = excluded.target, written_ms = excluded.written_ms",
                )
                .map_err(|error| error.to_string())?;
            for (location, target) in written {
                upsert
                    .execute(params![location, target, now])
                    .map_err(|error| error.to_string())?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())
    }
}

/// Renderer event after the open project's workspace was reloaded because
/// an `aeria` command wrote it.
pub const WORKSPACE_RELOADED_EVENT: &str = "project://workspace-reloaded";
/// How often the desktop looks for command writes.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Watches the open project for writes of `aeria` commands and reloads its
/// workspace after one, so the editor shows agents' translations while they
/// work.
pub(crate) fn start_reload_watcher(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};

    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(WATCH_INTERVAL);
            let state = app.state::<crate::state::DesktopState>();
            let Some(root) = state.lock_project().ok().and_then(|project| {
                project
                    .as_ref()
                    .map(|session| session.repository_root().to_owned())
            }) else {
                continue;
            };
            let Some(files) = state.project_files(&root) else {
                continue;
            };
            if !state.command_wrote(&files, &root) {
                continue;
            }
            // The cross-process lock first, then the project, as every write
            // takes them.
            let Ok(_lock) = WriteLock::acquire(&files) else {
                continue;
            };
            let reloaded = state.lock_project().ok().and_then(|mut project| {
                project
                    .as_mut()
                    .filter(|session| session.repository_root() == root)
                    .map(|session| state.take_in_command_writes(&files, session))
            });
            match reloaded {
                Some(Ok(true)) => {
                    let _ = app.emit(WORKSPACE_RELOADED_EVENT, ());
                }
                Some(Err(error)) => {
                    eprintln!(
                        "could not reload the project after an agent's write: {}",
                        error.message
                    );
                }
                _ => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_locks_and_the_ledger_are_shared_per_project() {
        let data = tempfile::tempdir().expect("data");
        let project = tempfile::tempdir().expect("project");
        let files = ProjectFiles::new(data.path(), project.path());
        assert_eq!(
            project_key(project.path()),
            project_key(&project.path().join("."))
        );
        assert_eq!(files.stamp(), None);
        {
            let _lock = WriteLock::acquire(&files).expect("lock");
            files.touch_stamp().expect("stamp");
        }
        let first = files.stamp().expect("stamped");
        let _again = WriteLock::acquire(&files).expect("released");

        let mut ledger = files.ledger().expect("ledger");
        assert_eq!(ledger.agent_target("Addon:1:0:1").expect("read"), None);
        ledger
            .record(&[("Addon:1:0:1".to_owned(), "ОК".to_owned())])
            .expect("record");
        ledger
            .record(&[("Addon:1:0:1".to_owned(), "Готово".to_owned())])
            .expect("again");
        let reopened = files.ledger().expect("reopen");
        assert_eq!(
            reopened
                .agent_target("Addon:1:0:1")
                .expect("read")
                .as_deref(),
            Some("Готово")
        );
        assert!(!first.is_empty());
    }
}
