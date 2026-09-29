use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use aeria_git::{GitExecutable, UnitAttribution};
use aeria_workspace::ProjectSession;

use crate::error::CommandError;
use crate::sync::{ProjectFiles, WriteLock};

type CommandResult<T> = Result<T, CommandError>;

/// The one authoritative project session owned by the desktop process.
pub struct DesktopState {
    project: Mutex<Option<ProjectSession>>,
    registry: Mutex<()>,
    git: OnceLock<GitExecutable>,
    attribution: Mutex<Option<AttributionCache>>,
    /// Aeria's data folder, where the files shared with `aeria` commands
    /// live; unset in tests, which share nothing.
    data_dir: OnceLock<PathBuf>,
    /// The project root and the stamp of the last command write the open
    /// session has taken in (see [`crate::sync`]).
    seen_stamp: Mutex<Option<(PathBuf, Option<String>)>>,
    /// Running synchronization and export operations, which an application
    /// update must not interrupt.
    activities: Mutex<Vec<(u64, Activity)>>,
    next_activity_id: AtomicU64,
}

/// Work that an application update waits for instead of interrupting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    /// Git operations that fetch, push, commit, or change the working tree.
    Sync,
    /// Pack export and publication.
    Export,
}

/// Marks an activity as running until dropped.
pub(crate) struct ActivityGuard<'a> {
    state: &'a DesktopState,
    id: u64,
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.state
            .lock_activities()
            .retain(|(id, _)| *id != self.id);
    }
}

/// Committed unit attribution for one repository commit. It is derived from
/// Git history, so it is disposable and keyed by the exact `HEAD`.
pub(crate) struct AttributionCache {
    pub root: PathBuf,
    pub head: String,
    pub units: Arc<Vec<UnitAttribution>>,
}

impl DesktopState {
    /// Creates an application state with no project open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            project: Mutex::new(None),
            registry: Mutex::new(()),
            git: OnceLock::new(),
            attribution: Mutex::new(None),
            data_dir: OnceLock::new(),
            seen_stamp: Mutex::new(None),
            activities: Mutex::new(Vec::new()),
            next_activity_id: AtomicU64::new(1),
        }
    }

    /// Records a synchronization or export operation until the guard drops.
    pub(crate) fn begin_activity(&self, activity: Activity) -> ActivityGuard<'_> {
        let id = self.next_activity_id.fetch_add(1, Ordering::Relaxed);
        self.lock_activities().push((id, activity));
        ActivityGuard { state: self, id }
    }

    /// Returns the running activities, each once, in a stable order.
    pub(crate) fn running_activities(&self) -> Vec<Activity> {
        let activities = self.lock_activities();
        Self::running_with(&activities)
    }

    /// Runs `operation` only when no activity runs. Synchronization and
    /// export cannot start until it returns.
    ///
    /// # Errors
    ///
    /// Returns `updateBusy`, naming the running activities, without running
    /// `operation`.
    pub(crate) fn while_idle<T>(&self, operation: impl FnOnce() -> T) -> Result<T, CommandError> {
        let activities = self.lock_activities();
        let running = Self::running_with(&activities);
        if !running.is_empty() {
            let names: Vec<_> = running
                .iter()
                .map(|activity| format!("{activity:?}"))
                .collect();
            return Err(CommandError::new(
                "updateBusy",
                format!("waiting for running work to finish: {}", names.join(", ")),
            ));
        }
        Ok(operation())
    }

    fn lock_activities(&self) -> MutexGuard<'_, Vec<(u64, Activity)>> {
        self.activities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn running_with(tracked: &[(u64, Activity)]) -> Vec<Activity> {
        let tracks = |wanted: Activity| tracked.iter().any(|(_, activity)| *activity == wanted);
        [
            (Activity::Sync, tracks(Activity::Sync)),
            (Activity::Export, tracks(Activity::Export)),
        ]
        .into_iter()
        .filter_map(|(activity, running)| running.then_some(activity))
        .collect()
    }

    /// Selects the Git executable once, at application setup.
    pub fn set_git(&self, git: GitExecutable) {
        let _ = self.git.set(git);
    }

    /// Returns the selected Git executable, or `git` from `PATH`.
    pub(crate) fn git(&self) -> GitExecutable {
        self.git
            .get()
            .cloned()
            .unwrap_or_else(GitExecutable::system)
    }

    pub(crate) fn cached_attribution(
        &self,
        root: &std::path::Path,
        head: &str,
    ) -> Option<Arc<Vec<UnitAttribution>>> {
        let cache = self.attribution.lock().ok()?;
        cache
            .as_ref()
            .filter(|cache| cache.root == root && cache.head == head)
            .map(|cache| Arc::clone(&cache.units))
    }

    pub(crate) fn store_attribution(&self, cache: AttributionCache) {
        if let Ok(mut current) = self.attribution.lock() {
            *current = Some(cache);
        }
    }

    pub(crate) fn lock_project(
        &self,
    ) -> Result<MutexGuard<'_, Option<ProjectSession>>, CommandError> {
        self.project
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project state lock is poisoned"))
    }

    /// Sets the data folder once, at application setup.
    pub(crate) fn set_data_dir(&self, data_dir: PathBuf) {
        let _ = self.data_dir.set(data_dir);
    }

    /// The shared files of the project at `root`, when the data folder is
    /// known.
    pub(crate) fn project_files(&self, root: &Path) -> Option<ProjectFiles> {
        self.data_dir
            .get()
            .map(|data_dir| ProjectFiles::new(data_dir, root))
    }

    /// Reloads the session's workspace when an `aeria` command wrote the
    /// project since the session last took its writes in. Returns whether
    /// it reloaded. Call while holding the project's [`WriteLock`].
    ///
    /// # Errors
    ///
    /// Returns the reload error; the session keeps its previous state.
    pub(crate) fn take_in_command_writes(
        &self,
        files: &ProjectFiles,
        session: &mut ProjectSession,
    ) -> CommandResult<bool> {
        let stamp = files.stamp();
        let root = session.repository_root().to_owned();
        let mut seen = self
            .seen_stamp
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match seen.as_ref() {
            Some((seen_root, seen_stamp)) if *seen_root == root && *seen_stamp == stamp => {
                return Ok(false);
            }
            _ => {}
        }
        session.reload_workspace().map_err(CommandError::from)?;
        *seen = Some((root, stamp));
        Ok(true)
    }

    /// Whether the stamp of the open project differs from the one its
    /// session took in, without taking the project lock for long.
    pub(crate) fn command_wrote(&self, files: &ProjectFiles, root: &Path) -> bool {
        let stamp = files.stamp();
        self.seen_stamp
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|(seen_root, seen_stamp)| seen_root == root && *seen_stamp != stamp)
    }

    /// Records the stamp a newly opened session read the workspace after,
    /// or forgets it when the project closes.
    pub(crate) fn reset_stamp(&self, root: Option<&Path>) {
        let seen = root.map(|root| {
            (
                root.to_owned(),
                self.project_files(root).and_then(|files| files.stamp()),
            )
        });
        *self
            .seen_stamp
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = seen;
    }

    /// Runs a workspace write on the open project: under the project's
    /// cross-process [`WriteLock`], after taking in writes of `aeria`
    /// commands.
    ///
    /// # Errors
    ///
    /// Returns `noProjectOpen`, `projectLock` when the lock cannot be taken,
    /// a reload error, or the write's own error.
    pub(crate) fn write_project<T>(
        &self,
        write: impl FnOnce(&mut ProjectSession) -> CommandResult<T>,
    ) -> CommandResult<T> {
        let root = {
            let project = self.lock_project()?;
            project
                .as_ref()
                .ok_or_else(CommandError::no_project)?
                .repository_root()
                .to_owned()
        };
        // The cross-process lock is always taken before the project mutex,
        // as the reload watcher does, so the two never wait on each other.
        let files = self.project_files(&root);
        let _lock = files
            .as_ref()
            .map(WriteLock::acquire)
            .transpose()
            .map_err(|error| CommandError::new("projectLock", error.to_string()))?;
        let mut project = self.lock_project()?;
        let session = project
            .as_mut()
            .filter(|session| session.repository_root() == root)
            .ok_or_else(CommandError::no_project)?;
        if let Some(files) = &files {
            self.take_in_command_writes(files, session)?;
        }
        let result = write(session)?;
        // `aeria` servers keep the workspace in memory too; the stamp tells
        // them to reload. The desktop's own stamp is taken in already.
        if let Some(files) = &files
            && files.touch_stamp().is_ok()
        {
            *self
                .seen_stamp
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((root, files.stamp()));
        }
        Ok(result)
    }

    pub(crate) fn lock_registry(&self) -> Result<MutexGuard<'_, ()>, CommandError> {
        self.registry
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project registry lock is poisoned"))
    }
}

impl Default for DesktopState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activities_are_reported_while_their_guards_live() {
        let state = DesktopState::new();
        assert!(state.running_activities().is_empty());
        let sync = state.begin_activity(Activity::Sync);
        {
            let _export = state.begin_activity(Activity::Export);
            let _second_sync = state.begin_activity(Activity::Sync);
            assert_eq!(
                state.running_activities(),
                [Activity::Sync, Activity::Export]
            );
        }
        assert_eq!(state.running_activities(), [Activity::Sync]);
        drop(sync);
        assert!(state.running_activities().is_empty());
    }

    #[test]
    fn idle_work_is_refused_while_an_activity_runs() {
        let state = DesktopState::new();
        let export = state.begin_activity(Activity::Export);
        let refused = state.while_idle(|| unreachable!("must not run while exporting"));
        assert_eq!(refused.expect_err("busy").code, "updateBusy");
        drop(export);
        assert_eq!(state.while_idle(|| 7).expect("idle"), 7);
    }
}
