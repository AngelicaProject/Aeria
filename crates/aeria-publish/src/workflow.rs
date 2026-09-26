//! The GitHub Actions workflow that builds the feed of a translation
//! repository from its release assets.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Where the workflow lives in the translation repository.
pub const FEED_WORKFLOW_PATH: &str = ".github/workflows/harmonia-feed.yml";

/// The workflow Aeria installs.
pub const FEED_WORKFLOW: &str = include_str!("../templates/harmonia-feed.yml");

/// Whether the repository has the workflow Aeria installs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowState {
    Missing,
    /// Identical to [`FEED_WORKFLOW`], ignoring line endings.
    Current,
    /// Changed by hand or installed by another Aeria version.
    Different,
}

/// # Errors
/// Returns the I/O error when the file exists but cannot be read.
pub fn feed_workflow_state(project_root: &Path) -> std::io::Result<WorkflowState> {
    match fs::read_to_string(project_root.join(FEED_WORKFLOW_PATH)) {
        Ok(text) if text.replace("\r\n", "\n") == FEED_WORKFLOW.replace("\r\n", "\n") => {
            Ok(WorkflowState::Current)
        }
        Ok(_) => Ok(WorkflowState::Different),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(WorkflowState::Missing),
        Err(error) => Err(error),
    }
}

/// Writes the workflow, replacing an existing file. Nothing is committed;
/// the workflow is a project file, committed by the next checkpoint.
///
/// # Errors
/// Returns the I/O error.
pub fn install_feed_workflow(project_root: &Path) -> std::io::Result<()> {
    let path = project_root.join(FEED_WORKFLOW_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("yml.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(FEED_WORKFLOW.replace("\r\n", "\n").as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_is_installed_and_recognized() {
        let temp = tempfile::tempdir().expect("temp");
        assert_eq!(
            feed_workflow_state(temp.path()).expect("state"),
            WorkflowState::Missing
        );
        install_feed_workflow(temp.path()).expect("install");
        assert_eq!(
            feed_workflow_state(temp.path()).expect("state"),
            WorkflowState::Current
        );
        fs::write(temp.path().join(FEED_WORKFLOW_PATH), "name: other\n").expect("edit");
        assert_eq!(
            feed_workflow_state(temp.path()).expect("state"),
            WorkflowState::Different
        );
    }

    #[test]
    fn workflow_reads_the_files_aeria_publishes() {
        for needle in [
            "aeria-pack.json",
            "feed-entry.json",
            "harmonia/([1-9][0-9]*)",
            "site/harmonia",
            "feed-v1.json",
            "signingKeyFingerprint",
            "\"format\": \"harmonia-feed\"",
        ] {
            assert!(FEED_WORKFLOW.contains(needle), "{needle}");
        }
    }

    #[test]
    fn workflow_deploys_only_from_the_default_branch() {
        for needle in [
            "gh workflow run harmonia-feed.yml",
            "--ref \"$DEFAULT_BRANCH\"",
            "github.ref_name == github.event.repository.default_branch",
        ] {
            assert!(FEED_WORKFLOW.contains(needle), "{needle}");
        }
        assert!(FEED_WORKFLOW_PATH.ends_with("/harmonia-feed.yml"));
    }
}
