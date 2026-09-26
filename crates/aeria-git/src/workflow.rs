//! The GitHub Actions workflow that validates pull requests with
//! `aeria-check` before they merge.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Where the workflow lives in the repository.
pub const CHECK_WORKFLOW_FILE: &str = ".github/workflows/aeria-check.yml";

const TEMPLATE: &str = include_str!("../templates/aeria-check.yml");

/// The `aeria-check` release a workflow installs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckRelease<'a> {
    /// The Aeria version that built it.
    pub version: &'a str,
    /// Where the `.tar.gz` with the binary is downloaded from.
    pub url: &'a str,
    /// SHA-256 of that archive, in lowercase hex.
    pub sha256: &'a str,
}

/// Whether the repository has the workflow this Aeria would install.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckWorkflowState {
    Missing,
    /// Identical to the rendered workflow, ignoring line endings.
    Current,
    /// Installed by another Aeria version or changed by hand.
    Different,
}

/// The workflow for a project in `project_prefix` (the project's folder
/// relative to the repository, empty or ending in `/`).
#[must_use]
pub fn render_check_workflow(release: &CheckRelease<'_>, project_prefix: &str) -> String {
    let project = project_prefix.trim_end_matches('/');
    let project = if project.is_empty() { "." } else { project };
    TEMPLATE
        .replace("\r\n", "\n")
        .replace("{{VERSION}}", release.version)
        .replace("{{URL}}", &yaml_string(release.url))
        .replace("{{SHA256}}", &yaml_string(release.sha256))
        .replace("{{PROJECT}}", &yaml_string(project))
}

/// A double-quoted YAML scalar.
fn yaml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// # Errors
/// Returns the I/O error when the file exists but cannot be read.
pub fn check_workflow_state(
    repository_root: &Path,
    rendered: &str,
) -> std::io::Result<CheckWorkflowState> {
    match fs::read_to_string(repository_root.join(CHECK_WORKFLOW_FILE)) {
        Ok(text) if text.replace("\r\n", "\n") == rendered => Ok(CheckWorkflowState::Current),
        Ok(_) => Ok(CheckWorkflowState::Different),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(CheckWorkflowState::Missing),
        Err(error) => Err(error),
    }
}

/// Writes the workflow, replacing an existing file. Nothing is committed;
/// the workflow is a project file, committed by the next checkpoint.
///
/// # Errors
/// Returns the I/O error.
pub fn install_check_workflow(repository_root: &Path, rendered: &str) -> std::io::Result<()> {
    let path = repository_root.join(CHECK_WORKFLOW_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("yml.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(rendered.as_bytes())?;
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

    const RELEASE: CheckRelease<'static> = CheckRelease {
        version: "0.2.0",
        url: "https://github.com/AngelicaProject/Aeria/releases/download/v0.2.0/aeria-check-0.2.0-x86_64-unknown-linux-gnu.tar.gz",
        sha256: "ab12",
    };

    #[test]
    fn the_template_is_filled_in() {
        let text = render_check_workflow(&RELEASE, "");
        for placeholder in ["{{VERSION}}", "{{URL}}", "{{SHA256}}", "{{PROJECT}}"] {
            assert!(!text.contains(placeholder), "{placeholder} is replaced");
        }
        assert!(text.contains("AERIA_CHECK_SHA256: \"ab12\""));
        assert!(text.contains("PROJECT: \".\""));
        assert!(text.contains("Install aeria-check 0.2.0"));
        assert!(
            render_check_workflow(&RELEASE, "translation/").contains("PROJECT: \"translation\"")
        );
    }

    #[test]
    fn the_workflow_is_installed_and_recognized() {
        let temp = tempfile::tempdir().expect("temp");
        let rendered = render_check_workflow(&RELEASE, "");
        assert_eq!(
            check_workflow_state(temp.path(), &rendered).expect("state"),
            CheckWorkflowState::Missing
        );
        install_check_workflow(temp.path(), &rendered).expect("install");
        assert_eq!(
            check_workflow_state(temp.path(), &rendered).expect("state"),
            CheckWorkflowState::Current
        );
        let newer = render_check_workflow(
            &CheckRelease {
                version: "0.3.0",
                ..RELEASE
            },
            "",
        );
        assert_eq!(
            check_workflow_state(temp.path(), &newer).expect("state"),
            CheckWorkflowState::Different
        );
    }
}
