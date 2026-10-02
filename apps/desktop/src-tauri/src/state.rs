use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use aeria_git::{GitExecutable, PendingCache};
use aeria_po::Session;

use crate::error::CommandError;

type CommandResult<T> = Result<T, CommandError>;

/// The one authoritative project session owned by the desktop process.
pub struct DesktopState {
    project: Mutex<Option<Arc<Session>>>,
    registry: Mutex<()>,
    git: OnceLock<GitExecutable>,
    /// Running synchronization and export operations, which an application
    /// update must not interrupt.
    activities: Mutex<Vec<(u64, Activity)>>,
    next_activity_id: AtomicU64,
    /// Uncommitted string changes already read, so the Git views can ask
    /// often without reading every changed file again.
    pending: Mutex<PendingCache>,
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

impl DesktopState {
    /// Creates an application state with no project open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            project: Mutex::new(None),
            registry: Mutex::new(()),
            git: OnceLock::new(),
            activities: Mutex::new(Vec::new()),
            next_activity_id: AtomicU64::new(1),
            pending: Mutex::new(PendingCache::new()),
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

    pub(crate) fn lock_project(&self) -> CommandResult<MutexGuard<'_, Option<Arc<Session>>>> {
        self.project
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project state lock is poisoned"))
    }

    /// The open project.
    ///
    /// # Errors
    ///
    /// Returns `noProjectOpen` when no project is open.
    /// The cache of uncommitted string changes. A poisoned lock is recovered
    /// with an empty cache, which is read again.
    pub(crate) fn pending_cache(&self) -> MutexGuard<'_, PendingCache> {
        self.pending.lock().unwrap_or_else(|poisoned| {
            let mut guard = poisoned.into_inner();
            *guard = PendingCache::default();
            guard
        })
    }

    pub(crate) fn session(&self) -> CommandResult<Arc<Session>> {
        self.lock_project()?
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(CommandError::no_project)
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
