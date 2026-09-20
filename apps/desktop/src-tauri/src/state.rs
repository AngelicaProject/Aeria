use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use aeria_atlas::{CancellationHandle, CancellationToken};
use aeria_workspace::ProjectSession;

use crate::error::CommandError;

/// The one authoritative project session owned by the desktop process.
pub struct DesktopState {
    project: Mutex<Option<ProjectSession>>,
    atlas_job: Mutex<Option<AtlasJob>>,
    next_atlas_job_id: AtomicU64,
}

struct AtlasJob {
    id: String,
    cancellation: CancellationHandle,
}

#[derive(Debug)]
pub(crate) struct StartedAtlasJob {
    pub id: String,
    pub token: CancellationToken,
}

impl DesktopState {
    /// Creates an application state with no project open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            project: Mutex::new(None),
            atlas_job: Mutex::new(None),
            next_atlas_job_id: AtomicU64::new(1),
        }
    }

    pub(crate) fn lock_project(
        &self,
    ) -> Result<MutexGuard<'_, Option<ProjectSession>>, CommandError> {
        self.project
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project state lock is poisoned"))
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
        });
        Ok(StartedAtlasJob { id, token })
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
        assert!(first.token.is_cancelled());
        state.finish_atlas_job(&first.id).expect("finish Atlas job");
        state
            .start_atlas_job()
            .expect("a later Atlas job can start");
    }
}
