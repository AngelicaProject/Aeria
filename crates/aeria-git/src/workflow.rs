//! The GitHub Actions workflow that guards the main branch with Aeria
//! Guard: it checks every pull request and push and summarizes the change
//! for review.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Where the workflow lives in the repository.
pub const GUARD_WORKFLOW_FILE: &str = ".github/workflows/aeria-guard.yml";

const TEMPLATE: &str = include_str!("../templates/aeria-guard.yml");

/// The Aeria Guard action a workflow runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuardAction<'a> {
    /// The Git revision of Aeria the action is taken from: a commit, so the
    /// workflow runs exactly that code, or a branch for development builds.
    pub reference: &'a str,
    /// The Aeria version of that revision, as a comment for people.
    pub version: &'a str,
}

/// Whether the repository has the workflow this Aeria would install.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardWorkflowState {
    Missing,
    /// Identical to the rendered workflow, ignoring line endings.
    Current,
    /// Installed by another Aeria version or changed by hand.
    Different,
}

/// The workflow guarding `branch` for a project in `project_prefix` (the
/// project's folder relative to the repository, empty or ending in `/`).
#[must_use]
pub fn render_guard_workflow(
    action: &GuardAction<'_>,
    branch: &str,
    project_prefix: &str,
) -> String {
    let project = project_prefix.trim_end_matches('/');
    let project = if project.is_empty() { "." } else { project };
    TEMPLATE
        .replace("\r\n", "\n")
        .replace("{{VERSION}}", action.version)
        .replace("{{REF}}", action.reference)
        .replace("{{BRANCH}}", &yaml_string(branch))
        .replace("{{PROJECT}}", &yaml_string(project))
}

/// A double-quoted YAML scalar.
fn yaml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// # Errors
/// Returns the I/O error when the file exists but cannot be read.
pub fn guard_workflow_state(
    repository_root: &Path,
    rendered: &str,
) -> std::io::Result<GuardWorkflowState> {
    match fs::read_to_string(repository_root.join(GUARD_WORKFLOW_FILE)) {
        Ok(text) if text.replace("\r\n", "\n") == rendered => Ok(GuardWorkflowState::Current),
        Ok(_) => Ok(GuardWorkflowState::Different),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(GuardWorkflowState::Missing),
        Err(error) => Err(error),
    }
}

/// Writes the workflow, replacing an existing file. Nothing is committed;
/// the workflow is a project file, committed by the next checkpoint.
///
/// # Errors
/// Returns the I/O error.
pub fn install_guard_workflow(repository_root: &Path, rendered: &str) -> std::io::Result<()> {
    let path = repository_root.join(GUARD_WORKFLOW_FILE);
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

    const ACTION: GuardAction<'static> = GuardAction {
        reference: "0123456789abcdef0123456789abcdef01234567",
        version: "0.2.0",
    };

    #[test]
    fn the_template_is_filled_in() {
        let text = render_guard_workflow(&ACTION, "main", "");
        for placeholder in ["{{VERSION}}", "{{REF}}", "{{BRANCH}}", "{{PROJECT}}"] {
            assert!(!text.contains(placeholder), "{placeholder} is replaced");
        }
        assert!(text.contains(
            "uses: AngelicaProject/Aeria/guard@0123456789abcdef0123456789abcdef01234567 # 0.2.0"
        ));
        assert!(text.contains("branches: [\"main\"]"));
        assert!(text.contains("project: \".\""));
        assert!(text.contains("name: Aeria Guard\n"));
        assert!(
            render_guard_workflow(&ACTION, "trunk", "translation/")
                .contains("project: \"translation\"")
        );
    }

    #[test]
    fn the_workflow_is_installed_and_recognized() {
        let temp = tempfile::tempdir().expect("temp");
        let rendered = render_guard_workflow(&ACTION, "main", "");
        assert_eq!(
            guard_workflow_state(temp.path(), &rendered).expect("state"),
            GuardWorkflowState::Missing
        );
        install_guard_workflow(temp.path(), &rendered).expect("install");
        assert_eq!(
            guard_workflow_state(temp.path(), &rendered).expect("state"),
            GuardWorkflowState::Current
        );
        let newer = render_guard_workflow(
            &GuardAction {
                version: "0.3.0",
                ..ACTION
            },
            "main",
            "",
        );
        assert_eq!(
            guard_workflow_state(temp.path(), &newer).expect("state"),
            GuardWorkflowState::Different
        );
    }
}
