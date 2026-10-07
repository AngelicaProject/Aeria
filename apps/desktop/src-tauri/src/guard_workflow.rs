//! Guarding a GitHub repository's main branch: the Aeria Guard workflow,
//! whether GitHub's rules protect the branch and the pack release tags, the
//! rulesets to import where they do not, and pull requests.
//!
//! The workflow names the Aeria Guard action by the commit this build is
//! made from, which the release build passes in as `AERIA_COMMIT`; a
//! development build names `main`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use aeria_git::{
    GuardAction, GuardWorkflowState, guard_workflow_state, install_guard_workflow,
    render_guard_workflow,
};
use aeria_publish::{GitHubClient, GitHubRepository, Protection, branch_ruleset, tag_ruleset};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::git::{open_repository, sync_remote};
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// How long the protection read from GitHub is reused: it is read without
/// signing in, which GitHub limits to 60 requests an hour.
const PROTECTION_TTL: Duration = Duration::from_mins(10);

/// The branch a workflow guards when the remote names none.
const FALLBACK_BRANCH: &str = "main";

fn action() -> GuardAction<'static> {
    GuardAction {
        reference: option_env!("AERIA_COMMIT")
            .filter(|value| !value.is_empty())
            .unwrap_or("main"),
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GuardWorkflowStateDto {
    Missing,
    Current,
    Different,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardWorkflowDto {
    /// The `origin` remote is a github.com repository.
    pub github: bool,
    /// The project is the repository's top folder, where GitHub looks for
    /// workflows.
    pub top_level: bool,
    pub state: GuardWorkflowStateDto,
    /// The branch the workflow guards: the remote's default branch.
    pub branch: String,
}

struct Context {
    repository: aeria_git::GitRepository,
    github: Option<GitHubRepository>,
    branch: String,
}

fn context(state: &DesktopState) -> CommandResult<Context> {
    let repository = open_repository(state)?;
    let github = repository
        .remotes()?
        .into_iter()
        .find(|remote| remote.name == "origin")
        .and_then(|remote| GitHubRepository::from_remote_url(&remote.url));
    let branch = repository
        .remote_default_branch()
        .ok()
        .flatten()
        .unwrap_or_else(|| FALLBACK_BRANCH.to_owned());
    Ok(Context {
        repository,
        github,
        branch,
    })
}

fn workflow(state: &DesktopState, install: bool) -> CommandResult<GuardWorkflowDto> {
    let Context {
        repository,
        github,
        branch,
    } = context(state)?;
    let top_level = repository.project_prefix().is_empty();
    let rendered = render_guard_workflow(&action(), &branch, "");
    if install {
        if !top_level {
            return Err(CommandError::new(
                "gitGuardWorkflowUnavailable",
                "GitHub reads workflows only from the repository's top folder, and this project is in a subfolder",
            ));
        }
        install_guard_workflow(repository.root(), &rendered).map_err(|error| {
            CommandError::new(
                "gitGuardWorkflowWrite",
                format!("cannot write the workflow: {error}"),
            )
        })?;
    }
    let state = match guard_workflow_state(repository.root(), &rendered) {
        Ok(GuardWorkflowState::Current) => GuardWorkflowStateDto::Current,
        Ok(GuardWorkflowState::Different) => GuardWorkflowStateDto::Different,
        Ok(GuardWorkflowState::Missing) | Err(_) => GuardWorkflowStateDto::Missing,
    };
    Ok(GuardWorkflowDto {
        github: github.is_some(),
        top_level,
        state,
        branch,
    })
}

#[tauri::command(rename_all = "camelCase")]
/// Whether the repository has the Aeria Guard workflow this build writes.
///
/// # Errors
///
/// Returns `noProjectOpen` or a Git error.
pub async fn git_guard_workflow(app: tauri::AppHandle) -> CommandResult<GuardWorkflowDto> {
    run_blocking(move || workflow(&app.state::<DesktopState>(), false)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Adds or updates `.github/workflows/aeria-guard.yml`. Nothing is
/// committed; it is a project file, committed like the others.
///
/// # Errors
///
/// Returns `gitGuardWorkflowUnavailable` for a project in a subfolder,
/// `gitGuardWorkflowWrite`, or a Git error.
pub async fn git_install_guard_workflow(app: tauri::AppHandle) -> CommandResult<GuardWorkflowDto> {
    run_blocking(move || workflow(&app.state::<DesktopState>(), true)).await
}

/// The protection last read, by repository and branch.
static PROTECTION: Mutex<Option<(String, Instant, Protection)>> = Mutex::new(None);

#[tauri::command(rename_all = "camelCase")]
/// How GitHub protects the main branch and the pack release tags, read
/// without signing in and reused for a few minutes; `refresh` reads again.
/// `None` when `origin` is not on github.com.
///
/// # Errors
///
/// Returns `noProjectOpen` or a Git error.
pub async fn git_repository_protection(
    app: tauri::AppHandle,
    refresh: bool,
) -> CommandResult<Option<Protection>> {
    let (github, branch) = run_blocking(move || {
        let context = context(&app.state::<DesktopState>())?;
        Ok((context.github, context.branch))
    })
    .await?;
    let Some(github) = github else {
        return Ok(None);
    };
    let key = format!("{}/{}@{branch}", github.owner, github.name);
    if !refresh
        && let Ok(cache) = PROTECTION.lock()
        && let Some((cached, read, protection)) = cache.as_ref()
        && *cached == key
        && read.elapsed() < PROTECTION_TTL
    {
        return Ok(Some(protection.clone()));
    }
    let client = GitHubClient::new()
        .map_err(|error| CommandError::new("gitHubUnavailable", error.to_string()))?;
    let protection = client.protection(&github, &branch).await;
    if let Ok(mut cache) = PROTECTION.lock() {
        *cache = Some((key, Instant::now(), protection.clone()));
    }
    Ok(Some(protection))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RulesetKind {
    Branch,
    Tags,
}

#[tauri::command(rename_all = "camelCase")]
/// Writes the ruleset to import on GitHub to `path`, which the person chose
/// in a save dialog.
///
/// # Errors
///
/// Returns `writeFailed` when the file cannot be written.
pub async fn git_save_ruleset(kind: RulesetKind, path: String) -> CommandResult<()> {
    run_blocking(move || {
        let ruleset = match kind {
            RulesetKind::Branch => branch_ruleset(),
            RulesetKind::Tags => tag_ruleset(),
        };
        let text = serde_json::to_string_pretty(&ruleset)
            .map_err(|error| CommandError::new("writeFailed", error.to_string()))?;
        std::fs::write(&path, format!("{text}\n")).map_err(|error| {
            CommandError::new("writeFailed", format!("cannot write {path}: {error}"))
        })
    })
    .await
}

/// Opens a page of the `origin` repository on GitHub.
async fn open_github_page(
    app: tauri::AppHandle,
    page: impl FnOnce(&GitHubRepository, &Context) -> CommandResult<String> + Send + 'static,
) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let state_app = app.clone();
    let url = run_blocking(move || {
        let context = context(&state_app.state::<DesktopState>())?;
        let github = context.github.as_ref().ok_or_else(|| {
            CommandError::new(
                "gitGuardWorkflowUnavailable",
                "the origin remote is not a github.com repository",
            )
        })?;
        page(github, &context)
    })
    .await?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| CommandError::new("openFailed", error.to_string()))
}

#[tauri::command(rename_all = "camelCase")]
/// Opens the repository's rulesets on GitHub, where rules are imported. The
/// address comes from the `origin` remote, never the renderer.
///
/// # Errors
///
/// Returns `gitGuardWorkflowUnavailable` when `origin` is not on github.com,
/// or `openFailed`.
pub async fn git_open_rules_settings(app: tauri::AppHandle) -> CommandResult<()> {
    open_github_page(app, |github, _| {
        Ok(format!("{}/settings/rules", github.homepage()))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Opens GitHub's page for a pull request from the current branch, on the
/// remote the branch syncs with. For a fork GitHub proposes the repository
/// it was forked from.
///
/// # Errors
///
/// Returns `gitGuardWorkflowUnavailable` when that remote is not on
/// github.com, `gitDetachedHead`, or `openFailed`.
pub async fn git_open_pull_request(app: tauri::AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let state_app = app.clone();
    let url = run_blocking(move || {
        let state = state_app.state::<DesktopState>();
        let repository = open_repository(&state)?;
        let branch = repository
            .status()?
            .branch
            .ok_or_else(|| CommandError::new("gitDetachedHead", "no branch is checked out"))?;
        let github = sync_remote(&repository)?
            .and_then(|remote| GitHubRepository::from_remote_url(&remote.url))
            .ok_or_else(|| {
                CommandError::new(
                    "gitGuardWorkflowUnavailable",
                    "the branch's remote is not a github.com repository",
                )
            })?;
        Ok(pull_request_url(&github, &branch))
    })
    .await?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| CommandError::new("openFailed", error.to_string()))
}

/// GitHub's page for a pull request from `branch`.
fn pull_request_url(github: &GitHubRepository, branch: &str) -> String {
    let branch: String = branch
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("{}/compare/{branch}?expand=1", github.homepage())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_requests_open_on_the_compare_page() {
        let github = GitHubRepository {
            owner: "translator".to_owned(),
            name: "ProjectPrima".to_owned(),
        };
        assert_eq!(
            pull_request_url(&github, "fix/квест #1"),
            "https://github.com/translator/ProjectPrima/compare/fix/%D0%BA%D0%B2%D0%B5%D1%81%D1%82%20%231?expand=1"
        );
    }

    #[test]
    fn development_builds_name_main() {
        if option_env!("AERIA_COMMIT").is_none() {
            assert_eq!(action().reference, "main");
        }
    }
}
