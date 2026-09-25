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

/// Project-shared collaboration settings. Work always reaches the main
/// branch through reviewed contribution branches (pull requests); the only
/// setting is which branch is the main branch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollaborationSettings {
    /// The branch contributions are reviewed into; `None` lets Aeria detect
    /// it (see `GitRepository::main_branch`).
    pub main_branch: Option<String>,
}

impl CollaborationSettings {
    /// Reads the settings from `project_root`. A missing file means a
    /// detected main branch.
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
        match object.get("policy").and_then(serde_json::Value::as_str) {
            Some("pull-request") => {}
            Some("direct") => {
                return Err(invalid(
                    "the direct policy is no longer supported: changes reach the main branch only through pull requests; set the policy to \"pull-request\"".to_owned(),
                ));
            }
            _ => return Err(invalid("policy must be \"pull-request\"".to_owned())),
        }
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
        Ok(Self { main_branch })
    }

    /// Returns the canonical file contents: two-space indented JSON with a
    /// final LF.
    #[must_use]
    pub fn to_canonical_json(&self) -> String {
        let policy = "pull-request";
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
        for main_branch in [Some("main".to_owned()), None] {
            let settings = CollaborationSettings { main_branch };
            let text = settings.to_canonical_json();
            assert!(
                text.starts_with("{\n  \"formatVersion\": 1,\n  \"policy\": \"pull-request\",\n")
            );
            assert_eq!(
                CollaborationSettings::parse(&text).expect("parse"),
                settings
            );
        }
    }

    #[test]
    fn invalid_newer_or_direct_settings_are_rejected() {
        for text in [
            "{\"formatVersion\":2,\"policy\":\"pull-request\"}",
            "{\"formatVersion\":1,\"policy\":\"review\"}",
            "{\"formatVersion\":1,\"policy\":\"direct\"}",
            "{\"formatVersion\":1,\"policy\":\"pull-request\",\"extra\":true}",
            "{\"formatVersion\":1,\"policy\":\"pull-request\",\"mainBranch\":\"\"}",
            "[]",
        ] {
            assert!(CollaborationSettings::parse(text).is_err(), "{text}");
        }
        let error = CollaborationSettings::parse("{\"formatVersion\":1,\"policy\":\"direct\"}")
            .expect_err("direct");
        assert!(error.to_string().contains("pull requests"), "{error}");
    }
}
