use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use aeria_ai::OpenAiCompatibleClient;
use aeria_atlas::{CancellationHandle, CancellationToken};
use aeria_git::{GitExecutable, UnitAttribution};
use aeria_workspace::ProjectSession;

use crate::error::CommandError;

/// The one authoritative project session owned by the desktop process.
pub struct DesktopState {
    project: Mutex<Option<ProjectSession>>,
    registry: Mutex<()>,
    atlas_job: Mutex<Option<AtlasJob>>,
    next_atlas_job_id: AtomicU64,
    git: OnceLock<GitExecutable>,
    attribution: Mutex<Option<AttributionCache>>,
    ai_client: OnceLock<OpenAiCompatibleClient>,
    angelica_turns: Mutex<Vec<(String, tauri::async_runtime::JoinHandle<()>)>>,
}

/// Committed unit attribution for one repository commit. It is derived from
/// Git history, so it is disposable and keyed by the exact `HEAD`.
pub(crate) struct AttributionCache {
    pub root: PathBuf,
    pub head: String,
    pub units: Arc<Vec<UnitAttribution>>,
}

struct AtlasJob {
    id: String,
    cancellation: CancellationHandle,
    token: CancellationToken,
}

#[derive(Debug)]
pub(crate) struct StartedAtlasJob {
    pub id: String,
}

impl DesktopState {
    /// Creates an application state with no project open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            project: Mutex::new(None),
            registry: Mutex::new(()),
            atlas_job: Mutex::new(None),
            next_atlas_job_id: AtomicU64::new(1),
            git: OnceLock::new(),
            attribution: Mutex::new(None),
            ai_client: OnceLock::new(),
            angelica_turns: Mutex::new(Vec::new()),
        }
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

    /// Returns the shared AI provider HTTP client, creating it on first use.
    /// The client is cheap to clone and holds no project state.
    pub(crate) fn ai_client(&self) -> Result<OpenAiCompatibleClient, CommandError> {
        if let Some(client) = self.ai_client.get() {
            return Ok(client.clone());
        }
        let client = OpenAiCompatibleClient::new()?;
        Ok(self.ai_client.get_or_init(|| client).clone())
    }

    /// Registers a conversation's running turn. The task is spawned while
    /// the registry is locked, so it cannot finish before it is registered.
    /// Returns `false`, without spawning, when the conversation is busy.
    pub(crate) fn start_angelica_turn(
        &self,
        conversation_id: String,
        spawn: impl FnOnce() -> tauri::async_runtime::JoinHandle<()>,
    ) -> bool {
        let Ok(mut turns) = self.angelica_turns.lock() else {
            return false;
        };
        if turns.iter().any(|(id, _)| *id == conversation_id) {
            return false;
        }
        turns.push((conversation_id, spawn()));
        true
    }

    pub(crate) fn angelica_turn_running(&self, conversation_id: &str) -> bool {
        self.angelica_turns
            .lock()
            .is_ok_and(|turns| turns.iter().any(|(id, _)| id == conversation_id))
    }

    /// Forgets a turn that ended on its own.
    pub(crate) fn finish_angelica_turn(&self, conversation_id: &str) {
        if let Ok(mut turns) = self.angelica_turns.lock() {
            turns.retain(|(id, _)| id != conversation_id);
        }
    }

    /// Stops a running turn. Returns whether one was running.
    pub(crate) fn cancel_angelica_turn(&self, conversation_id: &str) -> bool {
        let Ok(mut turns) = self.angelica_turns.lock() else {
            return false;
        };
        let Some(position) = turns.iter().position(|(id, _)| id == conversation_id) else {
            return false;
        };
        let (_, handle) = turns.remove(position);
        handle.abort();
        true
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

    pub(crate) fn lock_registry(&self) -> Result<MutexGuard<'_, ()>, CommandError> {
        self.registry
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project registry lock is poisoned"))
    }

    pub(crate) fn start_atlas_job(&self) -> Result<StartedAtlasJob, CommandError> {
        let mut active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        if active.is_some() {
            return Err(CommandError::new(
                "atlasBusy",
                "another source-package generation job is already active",
            ));
        }
        let sequence = self.next_atlas_job_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("atlas-{sequence:016x}");
        let (token, cancellation) = CancellationToken::new();
        *active = Some(AtlasJob {
            id: id.clone(),
            cancellation,
            token,
        });
        Ok(StartedAtlasJob { id })
    }

    pub(crate) fn cancel_atlas_job(&self, id: &str) -> Result<(), CommandError> {
        let active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        let job = active.as_ref().filter(|job| job.id == id).ok_or_else(|| {
            CommandError::new("atlasCancelled", "the requested Atlas job is not active")
        })?;
        job.cancellation.cancel();
        Ok(())
    }

    pub(crate) fn atlas_job_token(&self, id: &str) -> Result<CancellationToken, CommandError> {
        let active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        active
            .as_ref()
            .filter(|job| job.id == id)
            .map(|job| job.token.clone())
            .ok_or_else(|| {
                CommandError::new("atlasCancelled", "the requested Atlas job is not active")
            })
    }

    pub(crate) fn finish_atlas_job(&self, id: &str) -> Result<(), CommandError> {
        let mut active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        if active.as_ref().is_some_and(|job| job.id == id) {
            *active = None;
        }
        Ok(())
    }

    /// Runs the final Atlas project-publication boundary while holding the
    /// Atlas job slot. Cancellation requests cannot be accepted between the
    /// final cancellation check and the operation's project publication.
    pub(crate) fn with_atlas_publication<T, F>(
        &self,
        id: &str,
        token: &CancellationToken,
        operation: F,
    ) -> Result<T, CommandError>
    where
        F: FnOnce(&Self) -> Result<T, CommandError>,
    {
        let mut active = self.atlas_job.lock().map_err(|_| {
            CommandError::internal_state("desktop Atlas job state lock is poisoned")
        })?;
        if active.as_ref().is_none_or(|job| job.id != id) {
            return Err(CommandError::new(
                "atlasCancelled",
                "the requested Atlas job is not active",
            ));
        }
        if token.is_cancelled() {
            return Err(CommandError::new(
                "atlasCancelled",
                "Atlas project creation was cancelled",
            ));
        }

        let result = operation(self);
        if result.is_ok() {
            *active = None;
        }
        result
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
    fn desktop_state_allows_one_atlas_job_and_cancels_by_opaque_id() {
        let state = DesktopState::new();
        let first = state.start_atlas_job().expect("first Atlas job");
        let busy = state
            .start_atlas_job()
            .expect_err("second Atlas job is rejected");
        assert_eq!(busy.code, "atlasBusy");

        state
            .cancel_atlas_job(&first.id)
            .expect("active Atlas job cancels");
        assert!(
            state
                .atlas_job_token(&first.id)
                .expect("active Atlas job token")
                .is_cancelled()
        );
        state.finish_atlas_job(&first.id).expect("finish Atlas job");
        state
            .start_atlas_job()
            .expect("a later Atlas job can start");
    }

    #[test]
    fn atlas_job_token_is_only_available_for_the_active_job() {
        let state = DesktopState::new();
        let started = state.start_atlas_job().expect("Atlas job");

        assert!(state.atlas_job_token(&started.id).is_ok());
        assert_eq!(
            state
                .atlas_job_token("atlas-0000000000000002")
                .expect_err("stale job ID"),
            CommandError::new("atlasCancelled", "the requested Atlas job is not active")
        );
    }
}
