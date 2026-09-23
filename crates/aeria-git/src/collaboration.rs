//! Project-shared collaboration settings (Collaboration Settings v1).
//!
//! The settings file lives next to `.aeria/` in the project root and is
//! committed with the project so every collaborator uses the same policy.
//! See `docs/formats/collaboration-v1.md`.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::GitError;

/// The project-root file name of Collaboration Settings v1.
pub const COLLABORATION_FILE: &str = "aeria-collaboration.json";
const FORMAT_VERSION: u64 = 1;

/// How translators integrate their work.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CollaborationPolicy {
    /// Work on the current branch and push directly.
    #[default]
    Direct,
    /// Work on a contribution branch that is merged through review.
    PullRequest,
}

/// Project-shared collaboration settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollaborationSettings {
    pub policy: CollaborationPolicy,
    /// The branch contributions are reviewed into. Required for
    /// [`CollaborationPolicy::PullRequest`].
    pub main_branch: Option<String>,
}

impl CollaborationSettings {
    /// Reads the settings from `project_root`. A missing file means the
    /// default direct policy.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidSettings`] for invalid or newer settings and
    /// [`GitError::Io`] when the file cannot be read.
    pub fn load(project_root: &Path) -> Result<Self, GitError> {
        let path = project_root.join(COLLABORATION_FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(GitError::Io {
                    operation: "read collaboration settings",
                    path,
                    source,
                });
            }
        };
        Self::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
    }

    /// Parses Collaboration Settings v1 JSON.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidSettings`] for invalid or newer settings.
    pub fn parse(text: &str) -> Result<Self, GitError> {
        let invalid = |reason: String| GitError::InvalidSettings { reason };
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| invalid(error.to_string()))?;
        let object = value
            .as_object()
            .ok_or_else(|| invalid("expected a JSON object".to_owned()))?;
        for key in object.keys() {
            if !matches!(key.as_str(), "formatVersion" | "policy" | "mainBranch") {
                return Err(invalid(format!("unknown field {key:?}")));
            }
        }
        match object
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
        {
            Some(FORMAT_VERSION) => {}
            Some(version) => {
                return Err(invalid(format!(
                    "unsupported formatVersion {version}; update Aeria"
                )));
            }
            None => return Err(invalid("formatVersion must be the integer 1".to_owned())),
        }
        let policy = match object.get("policy").and_then(serde_json::Value::as_str) {
            Some("direct") => CollaborationPolicy::Direct,
            Some("pull-request") => CollaborationPolicy::PullRequest,
            _ => {
                return Err(invalid(
                    "policy must be \"direct\" or \"pull-request\"".to_owned(),
                ));
            }
        };
        let main_branch = match object.get("mainBranch") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(branch)) if !branch.trim().is_empty() => {
                Some(branch.clone())
            }
            Some(_) => {
                return Err(invalid(
                    "mainBranch must be a non-empty string or null".to_owned(),
                ));
            }
        };
        if policy == CollaborationPolicy::PullRequest && main_branch.is_none() {
            return Err(invalid(
                "the pull-request policy requires mainBranch".to_owned(),
            ));
        }
        Ok(Self {
            policy,
            main_branch,
        })
    }

    /// Returns the canonical file contents: two-space indented JSON with a
    /// final LF.
    #[must_use]
    pub fn to_canonical_json(&self) -> String {
        let policy = match self.policy {
            CollaborationPolicy::Direct => "direct",
            CollaborationPolicy::PullRequest => "pull-request",
        };
        let main_branch = self.main_branch.as_ref().map_or_else(
            || "null".to_owned(),
            |branch| serde_json::Value::from(branch.as_str()).to_string(),
        );
        format!(
            "{{\n  \"formatVersion\": {FORMAT_VERSION},\n  \"policy\": \"{policy}\",\n  \"mainBranch\": {main_branch}\n}}\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_canonically() {
        let settings = CollaborationSettings {
            policy: CollaborationPolicy::PullRequest,
            main_branch: Some("main".to_owned()),
        };
        let text = settings.to_canonical_json();
        assert_eq!(
            text,
            "{\n  \"formatVersion\": 1,\n  \"policy\": \"pull-request\",\n  \"mainBranch\": \"main\"\n}\n"
        );
        assert_eq!(
            CollaborationSettings::parse(&text).expect("parse"),
            settings
        );
    }

    #[test]
    fn invalid_or_newer_settings_are_rejected() {
        for text in [
            "{\"formatVersion\":2,\"policy\":\"direct\"}",
            "{\"formatVersion\":1,\"policy\":\"review\"}",
            "{\"formatVersion\":1,\"policy\":\"pull-request\"}",
            "{\"formatVersion\":1,\"policy\":\"direct\",\"extra\":true}",
            "[]",
        ] {
            assert!(CollaborationSettings::parse(text).is_err(), "{text}");
        }
    }
}
