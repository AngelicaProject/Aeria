use aeria_ai::{AiSettingsError, ProviderError, SecretStoreError};
use aeria_core::TranslationUnitIdParseError;
use aeria_git::GitError;
use aeria_projects::RegistryError;
use aeria_source::SourceError;
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

    pub(crate) fn registry_read(error: &RegistryError) -> Self {
        let code = match &error {
            RegistryError::UnsupportedVersion { .. } => "projectRegistryVersion",
            RegistryError::EntryNotFound { .. } => "recentProjectNotFound",
            RegistryError::Io { .. }
            | RegistryError::InvalidJson { .. }
            | RegistryError::InvalidData { .. }
            | RegistryError::Serialization { .. }
            | RegistryError::AtomicPublication { .. } => "projectRegistryRead",
        };
        Self::new(code, error.to_string())
    }

    pub(crate) fn registry_write(error: &RegistryError) -> Self {
        let code = match &error {
            RegistryError::UnsupportedVersion { .. } => "projectRegistryVersion",
            RegistryError::EntryNotFound { .. } => "recentProjectNotFound",
            RegistryError::Io { .. }
            | RegistryError::InvalidJson { .. }
            | RegistryError::InvalidData { .. }
            | RegistryError::Serialization { .. }
            | RegistryError::AtomicPublication { .. } => "projectRegistryWrite",
        };
        Self::new(code, error.to_string())
    }

    pub(crate) fn recent_project(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, message)
    }
}

impl From<RegistryError> for CommandError {
    fn from(error: RegistryError) -> Self {
        Self::registry_read(&error)
    }
}

impl From<AiSettingsError> for CommandError {
    fn from(error: AiSettingsError) -> Self {
        let code = match &error {
            AiSettingsError::Rejected { .. } => "aiInvalidSettings",
            AiSettingsError::UnsupportedVersion { .. } => "aiSettingsVersion",
            AiSettingsError::InvalidJson { .. } | AiSettingsError::InvalidData { .. } => {
                "aiSettingsCorrupt"
            }
            AiSettingsError::Io { .. }
            | AiSettingsError::Serialization { .. }
            | AiSettingsError::AtomicPublication { .. } => "aiSettingsStorage",
        };
        Self::new(code, error.to_string())
    }
}

impl From<SecretStoreError> for CommandError {
    fn from(error: SecretStoreError) -> Self {
        let code = match &error {
            SecretStoreError::InvalidKey { .. } => "aiInvalidApiKey",
            SecretStoreError::Unavailable { .. } => "aiSecretStoreUnavailable",
            SecretStoreError::Failed { .. } => "aiSecretStore",
        };
        Self::new(code, error.to_string())
    }
}

impl From<ProviderError> for CommandError {
    fn from(error: ProviderError) -> Self {
        let code = match &error {
            ProviderError::Network { .. } => "aiNetwork",
            ProviderError::Timeout => "aiTimeout",
            ProviderError::Unauthorized { .. } => "aiUnauthorized",
            ProviderError::NotFound { .. } => "aiEndpointNotFound",
            ProviderError::RateLimited { .. } => "aiRateLimited",
            ProviderError::Rejected { .. } => "aiRequestRejected",
            ProviderError::Unavailable { .. } => "aiProviderUnavailable",
            ProviderError::InvalidResponse { .. } => "aiInvalidResponse",
            ProviderError::Client { .. } => "aiClient",
        };
        Self::new(code, error.to_string())
    }
}

impl From<SourceError> for CommandError {
    fn from(error: SourceError) -> Self {
        Self::new("gameRead", error.to_string())
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
            ProjectSessionError::Store { .. } => "projectStore",
            ProjectSessionError::GameOutdated { .. } => "gameOutdated",
            ProjectSessionError::Compatibility { .. } => "projectCompatibility",
            ProjectSessionError::SourceUpdateRequired { .. } => "sourceUpdateRequired",
            ProjectSessionError::SourceUpdate { .. } => "sourceUpdate",
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
            TranslationMutationError::EmptyTarget => "emptyTranslationTarget",
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
        WorkspaceError::Source(_) | WorkspaceError::SourceCellNotFound { .. } => "translationRead",
        WorkspaceError::InvalidTarget { .. }
        | WorkspaceError::UnitNotFound { .. }
        | WorkspaceError::DuplicateUnitId { .. }
        | WorkspaceError::DuplicateSourceBinding { .. }
        | WorkspaceError::DetachedUnit { .. }
        | WorkspaceError::InvalidMetadata(_)
        | WorkspaceError::SourceLanguageMismatch { .. }
        | WorkspaceError::Identity(_) => "translationWorkspace",
    }
}

impl From<GitError> for CommandError {
    fn from(error: GitError) -> Self {
        let code = match &error {
            GitError::GitUnavailable { .. } => "gitUnavailable",
            GitError::CommandFailed { .. } => "gitCommandFailed",
            GitError::Parse { .. } => "gitProtocol",
            GitError::NotARepository { .. } => "gitNotRepository",
            GitError::AlreadyARepository { .. } => "gitAlreadyRepository",
            GitError::InvalidInput { .. } => "gitInvalidInput",
            GitError::IdentityMissing => "gitIdentityMissing",
            GitError::NothingToCommit => "gitNothingToCommit",
            GitError::UncommittedTranslations => "gitUncommittedTranslations",
            GitError::DetachedHead => "gitDetachedHead",
            GitError::UnbornHead => "gitNoCommits",
            GitError::MergeInProgress => "gitMergeInProgress",
            GitError::NoRemote => "gitNoRemote",
            GitError::MergeConflict { .. } => "gitMergeConflict",
            GitError::IncomingRejected { .. } => "gitIncomingRejected",
            GitError::TranslationConflicts { .. } => "gitTranslationConflicts",
            GitError::InvalidSettings { .. } => "gitInvalidSettings",
            GitError::MainBranchProtected { .. } => "gitMainBranchProtected",
            GitError::Workspace(_) => "gitWorkspaceData",
            GitError::Io { .. } => "gitIo",
        };
        Self::new(code, error.to_string())
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
    use aeria_workspace::{TranslationReadError, WorkspaceError};

    use super::*;

    #[test]
    fn game_failures_use_stable_codes() {
        let missing = aeria_source::GameSource::open(
            PathBuf::from("missing-game"),
            aeria_source::SourceLanguage::English,
        )
        .expect_err("missing game");
        assert_eq!(CommandError::from(missing).code, "gameRead");
        let outdated = CommandError::from(ProjectSessionError::GameOutdated {
            repository_root: PathBuf::from("repository"),
            project: "2026.10.01.0000.0000".parse().expect("version"),
            game: "2026.09.15.0000.0000".parse().expect("version"),
        });
        assert_eq!(outdated.code, "gameOutdated");
    }

    #[test]
    fn representative_backend_errors_use_boundary_codes() {
        let read_error =
            CommandError::from(TranslationReadError::InvalidPageLimit { limit: 0, max: 256 });
        assert_eq!(read_error.code, "translationRead");

        let empty_target_error = CommandError::from(TranslationMutationError::EmptyTarget);
        assert_eq!(empty_target_error.code, "emptyTranslationTarget");

        let mutation_error = CommandError::from(TranslationMutationError::Workspace(
            WorkspaceError::UnitNotFound {
                id: TranslationUnitId::from_bytes([0; 16]),
            },
        ));
        assert_eq!(mutation_error.code, "translationWorkspace");

        let blocked_error = CommandError::from(TranslationMutationError::SourceNotTranslatable {
            source_binding: aeria_core::SourceBinding::new("Synthetic", 42, 0, 0),
        });
        assert_eq!(blocked_error.code, "sourceNotTranslatable");
    }

    #[test]
    fn source_update_errors_use_stable_codes() {
        let required = CommandError::from(ProjectSessionError::SourceUpdateRequired {
            repository_root: PathBuf::from("repository"),
            requirement: aeria_workspace::SourceUpdateRequirement::SourceFactsMismatch { units: 1 },
        });
        assert_eq!(required.code, "sourceUpdateRequired");

        let detached = CommandError::from(TranslationMutationError::Workspace(
            WorkspaceError::DetachedUnit {
                id: TranslationUnitId::from_bytes([0; 16]),
            },
        ));
        assert_eq!(detached.code, "translationWorkspace");
    }
}
