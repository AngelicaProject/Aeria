//! One project written by several processes on this computer: the desktop
//! and any number of `aeria` commands run by agents.
//!
//! Per project, keyed by a hash of its canonical root, application data holds
//! `agents/<key>.lock` and `agents/<key>.stamp`:
//!
//! - Every workspace write takes an exclusive lock on the lock file for its
//!   whole run, so writes from different processes never interleave.
//! - A command that wrote translations replaces the stamp before it releases
//!   the lock. A process that keeps the workspace in memory compares the stamp
//!   with the one it last saw and reloads the workspace when it changed.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

const AGENTS_DIRECTORY: &str = "agents";

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
}

impl ProjectFiles {
    #[must_use]
    pub(crate) fn new(data_dir: &Path, root: &Path) -> Self {
        let directory = data_dir.join(AGENTS_DIRECTORY);
        let key = project_key(root);
        Self {
            lock: directory.join(format!("{key}.lock")),
            stamp: directory.join(format!("{key}.stamp")),
        }
    }

    /// Another file of the project in the same folder, such as
    /// `<key>.server`.
    #[must_use]
    pub(crate) fn path_with_extension(&self, extension: &str) -> PathBuf {
        self.lock.with_extension(extension)
    }

    /// The stamp of the last write by a command, if any.
    #[must_use]
    pub(crate) fn stamp(&self) -> Option<String> {
        retry_busy(|| fs::read_to_string(&self.stamp)).ok()
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
        let partial = self
            .stamp
            .with_extension(format!("stamp.{}", std::process::id()));
        fs::write(&partial, format!("{now}-{}", std::process::id()))?;
        replace_file(&partial, &self.stamp)
    }
}

/// Runs a file operation again while Windows refuses it because another
/// process has the file open right now (access denied, or a sharing or lock
/// violation), for about two seconds at most.
fn retry_busy<T>(mut operation: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut attempt = 1_u32;
    loop {
        match operation() {
            Err(error)
                if attempt < 20
                    && (matches!(error.raw_os_error(), Some(5 | 32 | 33))
                        || error.kind() == std::io::ErrorKind::PermissionDenied) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(
                    25 * u64::from(attempt.min(4)),
                ));
                attempt += 1;
            }
            result => return result,
        }
    }
}

/// Replaces `target` with `staged` atomically, waiting out a process that
/// briefly holds `target`.
///
/// # Errors
///
/// Returns the I/O error of the last attempt.
pub(crate) fn replace_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    retry_busy(|| fs::rename(staged, target)).inspect_err(|_| {
        let _ = fs::remove_file(staged);
    })
}

/// An exclusive lock on a file, released when dropped.
#[derive(Debug)]
pub(crate) struct FileLock {
    _file: File,
}

impl FileLock {
    /// Waits for and takes the lock on `path`, creating the file.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file cannot be opened or locked.
    pub(crate) fn acquire(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        file.lock()?;
        Ok(Self { _file: file })
    }
}

/// An exclusive lock on a project's workspace writes, released when
/// dropped.
#[derive(Debug)]
pub(crate) struct WriteLock {
    _lock: FileLock,
}

impl WriteLock {
    /// Waits for and takes the lock.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the lock file cannot be opened or locked.
    pub(crate) fn acquire(files: &ProjectFiles) -> std::io::Result<Self> {
        Ok(Self {
            _lock: FileLock::acquire(&files.lock)?,
        })
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
    fn stamps_and_locks_are_shared_per_project() {
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

        assert!(!first.is_empty());
    }
}
