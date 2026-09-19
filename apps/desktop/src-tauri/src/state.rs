use std::sync::{Mutex, MutexGuard};

use aeria_workspace::ProjectSession;

use crate::error::CommandError;

/// The one authoritative project session owned by the desktop process.
pub struct DesktopState {
    project: Mutex<Option<ProjectSession>>,
}

impl DesktopState {
    /// Creates an application state with no project open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            project: Mutex::new(None),
        }
    }

    pub(crate) fn lock_project(
        &self,
    ) -> Result<MutexGuard<'_, Option<ProjectSession>>, CommandError> {
        self.project
            .lock()
            .map_err(|_| CommandError::internal_state("desktop project state lock is poisoned"))
    }
}

impl Default for DesktopState {
    fn default() -> Self {
        Self::new()
    }
}
