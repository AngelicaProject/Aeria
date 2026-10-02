use aeria_git::GitError;
use aeria_po::{EditError, OpenError, ProjectError};
use aeria_projects::RegistryError;
use aeria_source::SourceError;
use serde::{Deserialize, Serialize};

use crate::dto::IssueDto;

/// A typed error returned by every application command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
    /// The problems of a refused translation, for the renderer to word in
    /// its language; `message` has them in English.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<IssueDto>,
}

impl CommandError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            issues: Vec::new(),
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

impl From<SourceError> for CommandError {
    fn from(error: SourceError) -> Self {
        Self::new("gameRead", error.to_string())
    }
}

impl From<ProjectError> for CommandError {
    fn from(error: ProjectError) -> Self {
        let code = match &error {
            ProjectError::Exists(_) => "projectExists",
            ProjectError::Missing { .. } => "projectMissing",
            ProjectError::Generate(_) => "gameRead",
            ProjectError::Io { .. } | ProjectError::Settings { .. } => "projectStore",
        };
        Self::new(code, error.to_string())
    }
}

impl From<OpenError> for CommandError {
    fn from(error: OpenError) -> Self {
        match error {
            OpenError::Project(error) => error.into(),
            OpenError::SourceLanguage { .. } => {
                Self::new("projectCompatibility", error.to_string())
            }
            OpenError::GameOutdated { .. } => Self::new("gameOutdated", error.to_string()),
            OpenError::UpdateRequired { .. } => {
                Self::new("sourceUpdateRequired", error.to_string())
            }
            OpenError::Version(_) => Self::new("projectStore", error.to_string()),
        }
    }
}

impl From<EditError> for CommandError {
    fn from(error: EditError) -> Self {
        let code = match &error {
            EditError::Source(_) => "translationRead",
            EditError::Project(_) => "translationPersistence",
            EditError::NotAnEntry(_) => "sourceNotTranslatable",
            EditError::Broken { .. } => "projectFileBroken",
            EditError::SourceMismatch(_) => "translationSourceIntegrity",
            EditError::Invalid(_) => "translationInvalid",
        };
        let issues = match &error {
            EditError::Invalid(issues) => issues.iter().map(IssueDto::from).collect(),
            _ => Vec::new(),
        };
        Self {
            issues,
            ..Self::new(code, error.to_string())
        }
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
            GitError::Io { .. } => "gitIo",
        };
        Self::new(code, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn game_and_project_failures_use_stable_codes() {
        let missing = aeria_source::GameSource::open(
            PathBuf::from("missing-game"),
            aeria_source::SourceLanguage::English,
        )
        .expect_err("missing game");
        assert_eq!(CommandError::from(missing).code, "gameRead");
        let outdated = CommandError::from(OpenError::GameOutdated {
            project: "2026.10.01.0000.0000".to_owned(),
            game: "2026.09.15.0000.0000".to_owned(),
        });
        assert_eq!(outdated.code, "gameOutdated");
        let required = CommandError::from(OpenError::UpdateRequired {
            project: "2026.09.01.0000.0000".to_owned(),
            game: "2026.09.15.0000.0000".to_owned(),
        });
        assert_eq!(required.code, "sourceUpdateRequired");
    }

    #[test]
    fn edit_failures_use_stable_codes() {
        let invalid = CommandError::from(EditError::Invalid(vec![aeria_po::Issue::Structure(
            "broken macro".to_owned(),
        )]));
        assert_eq!(invalid.code, "translationInvalid");
        assert_eq!(invalid.message, "broken macro");
        let missing = CommandError::from(EditError::NotAnEntry("Addon:9:0:0".to_owned()));
        assert_eq!(missing.code, "sourceNotTranslatable");
    }
}
