//! Offers the merge-check workflow for GitHub repositories.
//!
//! The workflow runs `aeria-check` on every pull request. It pins the
//! `aeria-check` archive built together with this Aeria release by URL and
//! SHA-256, which the release build passes in as `AERIA_CHECK_URL` and
//! `AERIA_CHECK_SHA256`. Builds without them (development builds) cannot
//! offer it.

use aeria_git::{
    CheckRelease, CheckWorkflowState, check_workflow_state, install_check_workflow,
    render_check_workflow,
};
use aeria_publish::GitHubRepository;
use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::git::open_repository;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// The release this build's workflow installs, when the build knows one.
fn release() -> Option<CheckRelease<'static>> {
    Some(CheckRelease {
        version: env!("CARGO_PKG_VERSION"),
        url: option_env!("AERIA_CHECK_URL").filter(|value| !value.is_empty())?,
        sha256: option_env!("AERIA_CHECK_SHA256").filter(|value| !value.is_empty())?,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckWorkflowStateDto {
    Missing,
    Current,
    Different,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckWorkflowDto {
    /// The `origin` remote is a github.com repository.
    pub github: bool,
    /// The project is the repository's top folder, where GitHub looks for
    /// workflows.
    pub top_level: bool,
    /// This build can write the workflow (release builds only).
    pub available: bool,
    pub state: CheckWorkflowStateDto,
    /// Where branch protection is set up on GitHub.
    pub branch_settings_url: Option<String>,
}

fn workflow(state: &DesktopState, install: bool) -> CommandResult<CheckWorkflowDto> {
    let repository = open_repository(state)?;
    let github = repository
        .remotes()?
        .into_iter()
        .find(|remote| remote.name == "origin")
        .and_then(|remote| GitHubRepository::from_remote_url(&remote.url));
    let top_level = repository.project_prefix().is_empty();
    let rendered = release().map(|release| render_check_workflow(&release, ""));
    if install {
        let rendered = rendered.as_deref().ok_or_else(|| {
            CommandError::new(
                "gitCheckWorkflowUnavailable",
                "this build of Aeria has no published aeria-check to pin; use a release build",
            )
        })?;
        if !top_level {
            return Err(CommandError::new(
                "gitCheckWorkflowUnavailable",
                "GitHub reads workflows only from the repository's top folder, and this project is in a subfolder",
            ));
        }
        install_check_workflow(repository.root(), rendered).map_err(|error| {
            CommandError::new(
                "gitCheckWorkflowWrite",
                format!("cannot write the workflow: {error}"),
            )
        })?;
    }
    let state = match rendered.as_deref() {
        Some(rendered) => check_workflow_state(repository.root(), rendered),
        // Without a release to compare with, only presence is known.
        None => check_workflow_state(repository.root(), "").map(|state| match state {
            CheckWorkflowState::Missing => CheckWorkflowState::Missing,
            _ => CheckWorkflowState::Current,
        }),
    };
    let state = match state {
        Ok(CheckWorkflowState::Current) => CheckWorkflowStateDto::Current,
        Ok(CheckWorkflowState::Different) => CheckWorkflowStateDto::Different,
        Ok(CheckWorkflowState::Missing) | Err(_) => CheckWorkflowStateDto::Missing,
    };
    Ok(CheckWorkflowDto {
        branch_settings_url: github
            .as_ref()
            .map(|github| format!("{}/settings/branches", github.homepage())),
        github: github.is_some(),
        top_level,
        available: rendered.is_some(),
        state,
    })
}

#[tauri::command(rename_all = "camelCase")]
/// Whether the repository has the merge-check workflow this build offers.
///
/// # Errors
///
/// Returns `noProjectOpen` or a Git error.
pub async fn git_check_workflow(app: tauri::AppHandle) -> CommandResult<CheckWorkflowDto> {
    run_blocking(move || workflow(&app.state::<DesktopState>(), false)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Adds or updates `.github/workflows/aeria-check.yml`. Nothing is
/// committed; the next checkpoint commits it like other project files.
///
/// # Errors
///
/// Returns `gitCheckWorkflowUnavailable` for a development build or a
/// project in a subfolder, `gitCheckWorkflowWrite`, or a Git error.
pub async fn git_install_check_workflow(app: tauri::AppHandle) -> CommandResult<CheckWorkflowDto> {
    run_blocking(move || workflow(&app.state::<DesktopState>(), true)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Opens the repository's branch settings on GitHub, where the check is made
/// required. The address comes from the `origin` remote, never the renderer.
///
/// # Errors
///
/// Returns `gitCheckWorkflowUnavailable` when `origin` is not on github.com,
/// or `openFailed`.
pub async fn git_open_branch_settings(app: tauri::AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let state_app = app.clone();
    let workflow =
        run_blocking(move || workflow(&state_app.state::<DesktopState>(), false)).await?;
    let url = workflow.branch_settings_url.ok_or_else(|| {
        CommandError::new(
            "gitCheckWorkflowUnavailable",
            "the origin remote is not a github.com repository",
        )
    })?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| CommandError::new("openFailed", error.to_string()))
}
