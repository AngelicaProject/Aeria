use aeria_core::TranslationUnitIdParseError;
use aeria_workspace::{
    ProjectSessionError, TranslationMutationError, TranslationReadError, WorkspaceError,
    WorkspaceStoreError,
};
use serde::{Deserialize, Serialize};

/// A typed error returned by every application command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn no_project() -> Self {
        Self::new("noProjectOpen", "no project is currently open")
    }

    pub(crate) fn internal_state(message: impl Into<String>) -> Self {
        Self::new("internalState", message)
    }
}

impl From<TranslationUnitIdParseError> for CommandError {
    fn from(error: TranslationUnitIdParseError) -> Self {
        Self::new("invalidTranslationUnitId", error.to_string())
    }
}

impl From<ProjectSessionError> for CommandError {
    fn from(error: ProjectSessionError) -> Self {
        let code = match &error {
            ProjectSessionError::Source { .. } => "projectSource",
            ProjectSessionError::Store { .. } => "projectStore",
            ProjectSessionError::Compatibility { .. }
            | ProjectSessionError::BlockedWorkspaceUnit { .. } => "projectCompatibility",
            ProjectSessionError::Workspace { .. } => "projectWorkspace",
        };
        Self::new(code, error.to_string())
    }
}

impl From<TranslationReadError> for CommandError {
    fn from(error: TranslationReadError) -> Self {
        let code = match &error {
            TranslationReadError::WorkspaceSourceMismatch { .. } => "translationSourceIntegrity",
            TranslationReadError::InvalidPageLimit { .. }
            | TranslationReadError::CursorSheetMismatch { .. }
            | TranslationReadError::Source(_) => "translationRead",
        };
        Self::new(code, error.to_string())
    }
}

impl From<TranslationMutationError> for CommandError {
    fn from(error: TranslationMutationError) -> Self {
        let code = match &error {
            TranslationMutationError::SourceNotTranslatable { .. } => "sourceNotTranslatable",
            TranslationMutationError::Workspace(error) => workspace_error_code(error),
            TranslationMutationError::Persistence(_) => "translationPersistence",
            TranslationMutationError::SourceIntegrity { .. } => "translationSourceIntegrity",
        };
        Self::new(code, error.to_string())
    }
}

fn workspace_error_code(error: &WorkspaceError) -> &'static str {
    match error {
        WorkspaceError::Hxs(_) | WorkspaceError::SourceCellNotFound { .. } => "translationRead",
        WorkspaceError::InvalidTarget { .. }
        | WorkspaceError::UnitNotFound { .. }
        | WorkspaceError::DuplicateUnitId { .. }
        | WorkspaceError::DuplicateSourceBinding { .. }
        | WorkspaceError::InvalidMetadata(_)
        | WorkspaceError::SourceLanguageMismatch { .. }
        | WorkspaceError::SourceContentMismatch { .. }
        | WorkspaceError::SourceSnapshotMismatch { .. }
        | WorkspaceError::Identity(_) => "translationWorkspace",
    }
}

impl From<WorkspaceStoreError> for CommandError {
    fn from(error: WorkspaceStoreError) -> Self {
        Self::new("projectStore", error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use aeria_core::TranslationUnitId;
    use aeria_workspace::{ProjectSession, TranslationReadError, WorkspaceError};

    use super::*;

    #[test]
    fn project_source_failures_use_a_stable_code() {
        let error = ProjectSession::open(
            PathBuf::from("missing-repository"),
            PathBuf::from("missing-source.hsp"),
            PathBuf::from("missing-cache"),
        )
        .err()
        .expect("missing source");

        assert_eq!(CommandError::from(error).code, "projectSource");
    }

    #[test]
    fn representative_backend_errors_use_boundary_codes() {
        let read_error =
            CommandError::from(TranslationReadError::InvalidPageLimit { limit: 0, max: 256 });
        assert_eq!(read_error.code, "translationRead");

        let mutation_error = CommandError::from(TranslationMutationError::Workspace(
            WorkspaceError::UnitNotFound {
                id: TranslationUnitId::from_bytes([0; 32]),
            },
        ));
        assert_eq!(mutation_error.code, "translationWorkspace");

        let blocked_error = CommandError::from(TranslationMutationError::SourceNotTranslatable {
            source_binding: aeria_core::SourceBinding::new("Synthetic", 42, 0, 0),
        });
        assert_eq!(blocked_error.code, "sourceNotTranslatable");
    }
}
