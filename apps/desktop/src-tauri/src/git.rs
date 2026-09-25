//! Desktop adapter for Git collaboration commands.
//!
//! Git operations run against the active project's repository root. Commands
//! that change the working tree or read it while the editor could write
//! (checkpoint, integration, branch switches) hold the project lock; network
//! operations do not.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use aeria_core::TranslationUnit;
use aeria_git::{
    Attribution, BranchInfo, CheckpointOutcome, CollaborationSettings, CommitSummary, ConfigScope,
    ConflictResolution, ContributionStatus, ContributorSummary, FileChangeKind, FileStatus,
    GitError, GitExecutable, GitOrigin, GitRepository, IntegrateOutcome, RecordVersion, RemoteInfo,
    RepositoryStatus, TranslatorIdentity, UnitAttribution, UnitChange, UnitChangeKind,
    UnitConflict, UnitHistory, UnitRevision, summarize_changes, summarize_contributors,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::commands::{parse_translation_unit_id, run_blocking};
use crate::dto::{ReviewStateDto, SourceBindingDto};
use crate::error::CommandError;
use crate::project_changes::{self, ProjectChangeDto};
use crate::state::{Activity, AttributionCache, DesktopState};

type CommandResult<T> = Result<T, CommandError>;

/// Largest page accepted by history commands.
const MAX_HISTORY_PAGE: usize = 200;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOverviewDto {
    pub runtime: GitRuntimeDto,
    /// `None` when the project is not inside a Git repository.
    pub repository: Option<GitStatusDto>,
    pub identity: Option<TranslatorIdentityDto>,
    pub remotes: Vec<GitRemoteDto>,
    pub collaboration: Option<CollaborationDto>,
    /// Present under the pull-request policy.
    pub contribution: Option<ContributionDto>,
}

/// The Git executable Aeria uses.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRuntimeDto {
    /// `git --version` output, or `None` when Git cannot be run.
    pub version: Option<String>,
    pub origin: GitOriginDto,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitOriginDto {
    System,
    Bundled,
    Override,
}

impl From<GitOrigin> for GitOriginDto {
    fn from(origin: GitOrigin) -> Self {
        match origin {
            GitOrigin::System => Self::System,
            GitOrigin::Bundled => Self::Bundled,
            GitOrigin::Override => Self::Override,
        }
    }
}

/// Changes reach the main branch only through pull requests.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationDto {
    /// The main branch set in `aeria-collaboration.json`, if any.
    pub configured_main_branch: Option<String>,
    /// The main branch in effect: configured or detected.
    pub main_branch: Option<String>,
    /// Why the settings file cannot be used, when it exists but is invalid.
    pub error: Option<String>,
}

fn collaboration_dto(repository: &GitRepository) -> Result<CollaborationDto, GitError> {
    match repository.collaboration() {
        Ok(settings) => Ok(CollaborationDto {
            main_branch: repository.main_branch()?,
            configured_main_branch: settings.main_branch,
            error: None,
        }),
        Err(error @ GitError::InvalidSettings { .. }) => Ok(CollaborationDto {
            configured_main_branch: None,
            main_branch: None,
            error: Some(error.to_string()),
        }),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContributionDto {
    pub main_branch: String,
    pub branch: Option<String>,
    pub published: bool,
    pub unmerged_commits: u32,
    /// No remote: the contribution is merged locally instead of through a
    /// pull request.
    pub local: bool,
}

impl From<ContributionStatus> for ContributionDto {
    fn from(status: ContributionStatus) -> Self {
        Self {
            main_branch: status.main_branch,
            branch: status.branch,
            published: status.published,
            unmerged_commits: status.unmerged_commits,
            local: status.local,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusDto {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub merge_in_progress: bool,
    pub has_translation_changes: bool,
    pub files: Vec<GitFileDto>,
}

impl From<RepositoryStatus> for GitStatusDto {
    fn from(status: RepositoryStatus) -> Self {
        Self {
            has_translation_changes: status.has_translation_changes(),
            branch: status.branch,
            head: status.head,
            upstream: status.upstream,
            ahead: status.ahead,
            behind: status.behind,
            merge_in_progress: status.merge_in_progress,
            files: status.files.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitFileKindDto {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileDto {
    pub path: String,
    pub original_path: Option<String>,
    pub kind: GitFileKindDto,
    pub staged: bool,
    pub translation_data: bool,
}

impl From<FileStatus> for GitFileDto {
    fn from(file: FileStatus) -> Self {
        Self {
            translation_data: file.is_translation_data(),
            kind: match file.kind {
                FileChangeKind::Added => GitFileKindDto::Added,
                FileChangeKind::Modified => GitFileKindDto::Modified,
                FileChangeKind::Deleted => GitFileKindDto::Deleted,
                FileChangeKind::Renamed => GitFileKindDto::Renamed,
                FileChangeKind::Copied => GitFileKindDto::Copied,
                FileChangeKind::TypeChanged => GitFileKindDto::TypeChanged,
                FileChangeKind::Untracked => GitFileKindDto::Untracked,
                FileChangeKind::Conflicted => GitFileKindDto::Conflicted,
            },
            path: file.path,
            original_path: file.original_path,
            staged: file.staged,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitConfigScopeDto {
    System,
    Global,
    Repository,
    Worktree,
    Command,
}

impl From<ConfigScope> for GitConfigScopeDto {
    fn from(scope: ConfigScope) -> Self {
        match scope {
            ConfigScope::System => Self::System,
            ConfigScope::Global => Self::Global,
            ConfigScope::Repository => Self::Repository,
            ConfigScope::Worktree => Self::Worktree,
            ConfigScope::Command => Self::Command,
        }
    }
}

/// The translator identity is the Git author identity.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslatorIdentityDto {
    pub name: Option<String>,
    pub email: Option<String>,
    pub name_scope: Option<GitConfigScopeDto>,
    pub email_scope: Option<GitConfigScopeDto>,
}

impl From<TranslatorIdentity> for TranslatorIdentityDto {
    fn from(identity: TranslatorIdentity) -> Self {
        Self {
            name: identity.name,
            email: identity.email,
            name_scope: identity.name_scope.map(Into::into),
            email_scope: identity.email_scope.map(Into::into),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemoteDto {
    pub name: String,
    pub url: String,
}

impl From<RemoteInfo> for GitRemoteDto {
    fn from(remote: RemoteInfo) -> Self {
        Self {
            name: remote.name,
            url: remote.url,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitDto {
    pub id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Seconds since the Unix epoch.
    pub authored_at: i64,
    /// Branch and tag names at this commit (`HEAD -> main`, `tag: …`).
    pub refs: Vec<String>,
    pub subject: String,
}

impl From<CommitSummary> for GitCommitDto {
    fn from(commit: CommitSummary) -> Self {
        Self {
            id: commit.id,
            parents: commit.parents,
            author_name: commit.author_name,
            author_email: commit.author_email,
            authored_at: commit.authored_at,
            refs: commit.refs,
            subject: commit.subject,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitVersionDto {
    pub source_binding: SourceBindingDto,
    pub target_macro: String,
    pub review_state: ReviewStateDto,
    pub translator_note: Option<String>,
}

impl From<&TranslationUnit> for UnitVersionDto {
    fn from(unit: &TranslationUnit) -> Self {
        Self {
            source_binding: unit.source_binding().into(),
            target_macro: unit.target_macro().to_owned(),
            review_state: unit.review_state().into(),
            translator_note: unit.translator_note().map(str::to_owned),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnitChangeKindDto {
    Added,
    Modified,
    Removed,
}

impl From<UnitChangeKind> for UnitChangeKindDto {
    fn from(kind: UnitChangeKind) -> Self {
        match kind {
            UnitChangeKind::Added => Self::Added,
            UnitChangeKind::Modified => Self::Modified,
            UnitChangeKind::Removed => Self::Removed,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitChangeDto {
    pub translation_unit_id: String,
    pub kind: UnitChangeKindDto,
    pub before: Option<UnitVersionDto>,
    pub after: Option<UnitVersionDto>,
    pub target_changed: bool,
    pub review_changed: bool,
    pub note_changed: bool,
}

impl From<&UnitChange> for UnitChangeDto {
    fn from(change: &UnitChange) -> Self {
        Self {
            translation_unit_id: change.id.to_string(),
            kind: change.kind.into(),
            before: change.before.as_ref().map(Into::into),
            after: change.after.as_ref().map(Into::into),
            target_changed: change.target_changed(),
            review_changed: change.review_changed(),
            note_changed: change.note_changed(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum RecordVersionDto {
    Absent,
    Valid { unit: UnitVersionDto },
    Invalid { message: String },
}

impl From<&RecordVersion> for RecordVersionDto {
    fn from(version: &RecordVersion) -> Self {
        match version {
            RecordVersion::Absent => Self::Absent,
            RecordVersion::Valid(unit) => Self::Valid {
                unit: unit.as_ref().into(),
            },
            RecordVersion::Invalid { message } => Self::Invalid {
                message: message.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitRevisionDto {
    pub commit: GitCommitDto,
    pub kind: UnitChangeKindDto,
    pub before: RecordVersionDto,
    pub after: RecordVersionDto,
}

impl From<UnitRevision> for UnitRevisionDto {
    fn from(revision: UnitRevision) -> Self {
        Self {
            kind: revision.kind.into(),
            before: (&revision.before).into(),
            after: (&revision.after).into(),
            commit: revision.commit.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributionDto {
    pub commit: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
}

impl From<&Attribution> for AttributionDto {
    fn from(attribution: &Attribution) -> Self {
        Self {
            commit: attribution.commit.clone(),
            author_name: attribution.author_name.clone(),
            author_email: attribution.author_email.clone(),
            authored_at: attribution.authored_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitHistoryDto {
    pub translation_unit_id: String,
    pub pending: Option<UnitChangeDto>,
    pub revisions: Vec<UnitRevisionDto>,
    pub truncated: bool,
    pub translated_by: Option<AttributionDto>,
    pub reviewed_by: Option<AttributionDto>,
}

impl From<UnitHistory> for UnitHistoryDto {
    fn from(history: UnitHistory) -> Self {
        Self {
            translation_unit_id: history.id.to_string(),
            pending: history.pending.as_ref().map(Into::into),
            revisions: history.revisions.into_iter().map(Into::into).collect(),
            truncated: history.truncated,
            translated_by: history.translated_by.as_ref().map(Into::into),
            reviewed_by: history.reviewed_by.as_ref().map(Into::into),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitAttributionDto {
    pub translation_unit_id: String,
    pub translated_by: Option<AttributionDto>,
    pub reviewed_by: Option<AttributionDto>,
    pub last_changed_by: Option<AttributionDto>,
}

impl From<&UnitAttribution> for UnitAttributionDto {
    fn from(attribution: &UnitAttribution) -> Self {
        Self {
            translation_unit_id: attribution.id.to_string(),
            translated_by: attribution.translated_by.as_ref().map(Into::into),
            reviewed_by: attribution.reviewed_by.as_ref().map(Into::into),
            last_changed_by: attribution.last_changed_by.as_ref().map(Into::into),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitChangesDto {
    pub commit: GitCommitDto,
    pub changes: Vec<UnitChangeDto>,
    /// Glossary, guidance, settings, and font file changes.
    pub project_changes: Vec<ProjectChangeDto>,
    /// The contribution branch created by a checkpoint under the
    /// pull-request policy.
    pub branch_created: Option<String>,
}

impl From<CheckpointOutcome> for GitCommitChangesDto {
    fn from(outcome: CheckpointOutcome) -> Self {
        Self {
            commit: outcome.commit.into(),
            changes: outcome.changes.iter().map(Into::into).collect(),
            project_changes: Vec::new(),
            branch_created: outcome.branch_created,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContributorDto {
    pub name: String,
    pub email: String,
    pub translated: usize,
    pub reviewed: usize,
    pub last_authored_at: i64,
}

impl From<ContributorSummary> for ContributorDto {
    fn from(summary: ContributorSummary) -> Self {
        Self {
            name: summary.name,
            email: summary.email,
            translated: summary.translated,
            reviewed: summary.reviewed,
            last_authored_at: summary.last_authored_at,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitIntegrationDto {
    UpToDate,
    FastForward,
    Merged,
}

impl From<IntegrateOutcome> for GitIntegrationDto {
    fn from(outcome: IntegrateOutcome) -> Self {
        match outcome {
            IntegrateOutcome::UpToDate => Self::UpToDate,
            IntegrateOutcome::FastForward => Self::FastForward,
            IntegrateOutcome::Merged => Self::Merged,
        }
    }
}

/// One translation unit changed differently here and on the remote.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitConflictDto {
    pub translation_unit_id: String,
    pub base: Option<UnitVersionDto>,
    pub ours: Option<UnitVersionDto>,
    pub theirs: Option<UnitVersionDto>,
}

impl From<&UnitConflict> for UnitConflictDto {
    fn from(conflict: &UnitConflict) -> Self {
        Self {
            translation_unit_id: conflict.id.to_string(),
            base: conflict.base.as_ref().map(Into::into),
            ours: conflict.ours.as_ref().map(Into::into),
            theirs: conflict.theirs.as_ref().map(Into::into),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictResolutionDto {
    Ours,
    Theirs,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitResolutionDto {
    pub translation_unit_id: String,
    pub resolution: ConflictResolutionDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSyncDto {
    pub integration: GitIntegrationDto,
    pub pushed: bool,
    /// Whether the renderer must reload translation data.
    pub workspace_changed: bool,
    /// Same-unit conflicts. When non-empty nothing was integrated or pushed;
    /// sync again with a resolution for every conflict.
    pub conflicts: Vec<UnitConflictDto>,
    /// Integration left uncommitted changes: merged translations reconciled
    /// with the current game source. They wait for a checkpoint.
    pub reconciled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchDto {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: Option<String>,
    /// Every commit of the local branch is in the main branch.
    pub merged: bool,
    /// Why the open project cannot switch to this branch, or `None`.
    pub blocked: Option<BranchBlockDto>,
}

/// Why a branch holds a project the open session cannot load as is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchBlockDto {
    /// The branch has no Aeria project.
    NoProject,
    /// The branch stores an older Workspace Format that must be migrated.
    OlderFormat,
    /// The branch is bound to another game source.
    OtherSource,
}

/// Compares the workspace manifest a branch stores with the open session:
/// same Workspace Format and same source content. Unreadable manifests are
/// left to the switch itself, which validates everything.
fn branch_block(
    repository: &GitRepository,
    branch: &str,
    content_id: &str,
) -> Option<BranchBlockDto> {
    let revision = repository.branch_head(branch).ok().flatten()?;
    let Ok(manifest) = repository.file_at(&revision, ".aeria/manifest.json") else {
        return None;
    };
    let Some(manifest) = manifest else {
        return Some(BranchBlockDto::NoProject);
    };
    let value: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    if value
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(u64::from(aeria_workspace::WORKSPACE_FORMAT_VERSION))
    {
        return Some(BranchBlockDto::OlderFormat);
    }
    (value.get("contentId").and_then(serde_json::Value::as_str) != Some(content_id))
        .then_some(BranchBlockDto::OtherSource)
}

impl From<BranchInfo> for GitBranchDto {
    fn from(branch: BranchInfo) -> Self {
        Self {
            name: branch.name,
            remote: branch.remote,
            current: branch.current,
            upstream: branch.upstream,
            merged: false,
            blocked: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFinishDto {
    pub integration: GitIntegrationDto,
    pub deleted_branch: Option<String>,
}

fn project_root(state: &DesktopState) -> CommandResult<PathBuf> {
    let project = state.lock_project()?;
    let project = project.as_ref().ok_or_else(CommandError::no_project)?;
    Ok(project.repository_root().to_owned())
}

pub(crate) fn open_repository(state: &DesktopState) -> CommandResult<GitRepository> {
    Ok(GitRepository::open(project_root(state)?, state.git())?)
}

fn bounded_limit(limit: u32) -> CommandResult<usize> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if limit == 0 || limit > MAX_HISTORY_PAGE {
        return Err(CommandError::new(
            "gitInvalidInput",
            format!("history limit must be between 1 and {MAX_HISTORY_PAGE}"),
        ));
    }
    Ok(limit)
}

/// Selects the Git executable: the `AERIA_GIT_PATH` override, the `MinGit`
/// runtime bundled as `git/` in the application resources (Windows), or
/// `git` on `PATH`.
pub(crate) fn resolve_git(app: &tauri::AppHandle) -> GitExecutable {
    let bundled = if cfg!(windows) {
        app.path()
            .resource_dir()
            .ok()
            .map(|dir| dir.join("git").join("cmd").join("git.exe"))
    } else {
        None
    };
    GitExecutable::discover(bundled.as_deref())
}

/// Runs a Git operation that can change the working tree while holding the
/// project lock; `accept` reloads the session and reconciles merged units
/// that do not describe the session source (for example translations made
/// on a branch that had not applied the latest game update). The outer
/// result reports state errors, the inner one the Git operation.
fn with_session_reload<T>(
    state: &DesktopState,
    operation: impl FnOnce(
        &GitRepository,
        &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<T, GitError>,
) -> CommandResult<Result<T, GitError>> {
    let git = state.git();
    let mut project = state.lock_project()?;
    let session = project.as_mut().ok_or_else(CommandError::no_project)?;
    let repository = GitRepository::open(session.repository_root(), git)?;
    let mut accept = || {
        session
            .reload_and_reconcile_workspace()
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    Ok(operation(&repository, &mut accept))
}

pub(crate) fn git_overview_with_state(state: &DesktopState) -> CommandResult<GitOverviewDto> {
    let root = project_root(state)?;
    let git = state.git();
    let runtime = GitRuntimeDto {
        version: git.version(&root).ok(),
        origin: git.origin().into(),
    };
    let Some(repository) = GitRepository::discover(root, git)? else {
        return Ok(GitOverviewDto {
            runtime,
            repository: None,
            identity: None,
            remotes: Vec::new(),
            collaboration: None,
            contribution: None,
        });
    };
    Ok(GitOverviewDto {
        runtime,
        repository: Some(repository.status()?.into()),
        identity: Some(repository.identity()?.into()),
        remotes: repository.remotes()?.into_iter().map(Into::into).collect(),
        collaboration: Some(collaboration_dto(&repository)?),
        contribution: repository
            .contribution_status()
            .ok()
            .flatten()
            .map(Into::into),
    })
}

pub(crate) fn git_checkpoint_with_state(
    state: &DesktopState,
    message: Option<&str>,
) -> CommandResult<GitCommitChangesDto> {
    let git = state.git();
    let project = state.lock_project()?;
    let project = project.as_ref().ok_or_else(CommandError::no_project)?;
    let repository = GitRepository::open(project.repository_root(), git)?;
    let project_files = project_changes::pending(&repository, project.repository_root())?;
    let message = if let Some(message) = message.map(str::trim).filter(|text| !text.is_empty()) {
        Some(message.to_owned())
    } else {
        let units = repository.pending_changes()?;
        let translations = (!units.is_empty()).then(|| summarize_changes(&units));
        project_changes::checkpoint_message(translations.as_deref(), &project_files)
    };
    let mut outcome: GitCommitChangesDto = repository.checkpoint(message.as_deref())?.into();
    outcome.project_changes = project_files;
    Ok(outcome)
}

pub(crate) fn git_sync_with_state(
    state: &DesktopState,
    resolutions: &[UnitResolutionDto],
) -> CommandResult<GitSyncDto> {
    let resolutions = resolutions
        .iter()
        .map(|entry| {
            let resolution = match entry.resolution {
                ConflictResolutionDto::Ours => ConflictResolution::Ours,
                ConflictResolutionDto::Theirs => ConflictResolution::Theirs,
            };
            parse_translation_unit_id(&entry.translation_unit_id).map(|id| (id, resolution))
        })
        .collect::<CommandResult<BTreeMap<_, _>>>()?;

    let repository = open_repository(state)?;
    repository.fetch()?;
    let integration = with_session_reload(state, |repository, accept| {
        repository.integrate(&resolutions, accept)
    })?;
    let integration = match integration {
        Ok(integration) => integration,
        // Nothing was integrated or pushed; the repository is unchanged.
        Err(GitError::TranslationConflicts { conflicts }) => {
            return Ok(GitSyncDto {
                integration: GitIntegrationDto::UpToDate,
                pushed: false,
                workspace_changed: false,
                conflicts: conflicts.iter().map(Into::into).collect(),
                reconciled: false,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let pushed = repository.push()?;
    let reconciled =
        integration.changed_working_tree() && repository.status()?.has_translation_changes();
    Ok(GitSyncDto {
        integration: integration.into(),
        pushed,
        workspace_changed: integration.changed_working_tree(),
        conflicts: Vec::new(),
        reconciled,
    })
}

fn attribution_with_state(state: &DesktopState) -> CommandResult<Arc<Vec<UnitAttribution>>> {
    let repository = open_repository(state)?;
    let Some(head) = repository.head()? else {
        return Ok(Arc::new(Vec::new()));
    };
    if let Some(cached) = state.cached_attribution(repository.root(), &head) {
        return Ok(cached);
    }
    let units = Arc::new(repository.attribution()?);
    state.store_attribution(AttributionCache {
        root: repository.root().to_owned(),
        head,
        units: Arc::clone(&units),
    });
    Ok(units)
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the Git runtime, repository status, translator identity, remotes,
/// and collaboration policy for the active project.
///
/// # Errors
///
/// Returns a typed command error when no project is open or Git fails.
pub async fn git_overview(app: tauri::AppHandle) -> CommandResult<GitOverviewDto> {
    run_blocking(move || git_overview_with_state(&app.state::<DesktopState>())).await
}

#[tauri::command(rename_all = "camelCase")]
/// Initializes a Git repository at the active project root.
///
/// # Errors
///
/// Returns a typed command error when the project is already inside a
/// repository or Git fails.
pub async fn git_initialize(app: tauri::AppHandle) -> CommandResult<GitOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        GitRepository::init(project_root(&state)?, state.git())?;
        git_overview_with_state(&state)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Sets the translator (Git author) identity for this repository or globally.
/// The email is optional.
///
/// # Errors
///
/// Returns a typed command error for invalid values or a Git failure.
pub async fn git_set_identity(
    app: tauri::AppHandle,
    name: String,
    email: Option<String>,
    global: bool,
) -> CommandResult<TranslatorIdentityDto> {
    run_blocking(move || {
        let repository = open_repository(&app.state::<DesktopState>())?;
        repository.set_identity(&name, email.as_deref(), global)?;
        Ok(repository.identity()?.into())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Adds a remote or changes its URL.
///
/// # Errors
///
/// Returns a typed command error for an unsafe name or URL or a Git failure.
pub async fn git_set_remote(
    app: tauri::AppHandle,
    name: String,
    url: String,
) -> CommandResult<Vec<GitRemoteDto>> {
    run_blocking(move || {
        let repository = open_repository(&app.state::<DesktopState>())?;
        repository.set_remote(&name, &url)?;
        Ok(repository.remotes()?.into_iter().map(Into::into).collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns uncommitted translation-unit changes.
///
/// # Errors
///
/// Returns a typed command error when Git fails or workspace data is invalid.
pub async fn git_pending_changes(app: tauri::AppHandle) -> CommandResult<Vec<UnitChangeDto>> {
    run_blocking(move || {
        let repository = open_repository(&app.state::<DesktopState>())?;
        Ok(repository
            .pending_changes()?
            .iter()
            .map(Into::into)
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Commits Aeria-managed project data as the translator identity. A blank
/// message is replaced by a generated summary. Under the pull-request policy
/// a checkpoint on the main branch starts a contribution branch.
///
/// # Errors
///
/// Returns a typed command error when the name is missing, there is nothing
/// to commit, or Git fails.
pub async fn git_checkpoint(
    app: tauri::AppHandle,
    message: Option<String>,
) -> CommandResult<GitCommitChangesDto> {
    run_sync(app, move |state| {
        git_checkpoint_with_state(state, message.as_deref())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the uncommitted glossary, guidance, settings, and font file
/// changes, compared with `HEAD`.
///
/// # Errors
///
/// Returns a typed command error when no project is open or Git fails.
pub async fn git_project_changes(app: tauri::AppHandle) -> CommandResult<Vec<ProjectChangeDto>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let repository = open_repository(&state)?;
        let root = repository.root().to_owned();
        Ok(project_changes::pending(&repository, &root)?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns a fingerprint of the repository state for cheap polling; `None`
/// outside a repository.
///
/// # Errors
///
/// Returns a typed command error when no project is open or Git fails.
pub async fn git_state_stamp(app: tauri::AppHandle) -> CommandResult<Option<String>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let root = project_root(&state)?;
        match GitRepository::discover(root, state.git())? {
            Some(repository) => Ok(Some(repository.state_stamp()?)),
            None => Ok(None),
        }
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Removes a remote.
///
/// # Errors
///
/// Returns a typed command error for an invalid or unknown remote.
pub async fn git_remove_remote(
    app: tauri::AppHandle,
    name: String,
) -> CommandResult<GitOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        open_repository(&state)?.remove_remote(&name)?;
        git_overview_with_state(&state)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Fetches every remote and lists their branches, for choosing the upstream.
///
/// # Errors
///
/// Returns a typed command error when Git fails.
pub async fn git_remote_branches(app: tauri::AppHandle) -> CommandResult<Vec<String>> {
    run_blocking(move || {
        let repository = open_repository(&app.state::<DesktopState>())?;
        repository.fetch_all()?;
        Ok(repository.remote_branches()?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Sets the upstream (`<remote>/<branch>`) of the current branch.
///
/// # Errors
///
/// Returns a typed command error for an unknown remote branch.
pub async fn git_set_upstream(
    app: tauri::AppHandle,
    remote_branch: String,
) -> CommandResult<GitOverviewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        open_repository(&state)?.set_upstream(&remote_branch)?;
        git_overview_with_state(&state)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns project history, newest first.
///
/// # Errors
///
/// Returns a typed command error for an invalid page or a Git failure.
pub async fn git_log(
    app: tauri::AppHandle,
    skip: u32,
    limit: u32,
) -> CommandResult<Vec<GitCommitDto>> {
    run_blocking(move || {
        let limit = bounded_limit(limit)?;
        let repository = open_repository(&app.state::<DesktopState>())?;
        let skip = usize::try_from(skip).unwrap_or(usize::MAX);
        Ok(repository
            .log(skip, limit)?
            .into_iter()
            .map(Into::into)
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the translation-unit changes introduced by one commit.
///
/// # Errors
///
/// Returns a typed command error for an invalid commit ID or a Git failure.
pub async fn git_commit_changes(
    app: tauri::AppHandle,
    commit_id: String,
) -> CommandResult<GitCommitChangesDto> {
    run_sync(app, move |state| {
        let repository = open_repository(state)?;
        let (commit, changes) = repository.commit_changes(&commit_id)?;
        Ok(GitCommitChangesDto {
            project_changes: project_changes::of_commit(&repository, &commit.id)?,
            commit: commit.into(),
            changes: changes.iter().map(Into::into).collect(),
            branch_created: None,
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the history of one translation unit: who translated and reviewed
/// it, and every committed change.
///
/// # Errors
///
/// Returns a typed command error for an invalid ID or limit, or a Git failure.
pub async fn git_unit_history(
    app: tauri::AppHandle,
    translation_unit_id: String,
    limit: u32,
) -> CommandResult<UnitHistoryDto> {
    run_blocking(move || {
        let id = parse_translation_unit_id(&translation_unit_id)?;
        let limit = bounded_limit(limit)?;
        let repository = open_repository(&app.state::<DesktopState>())?;
        Ok(repository.unit_history(id, limit)?.into())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns committed attribution (translator, reviewer, last change) for the
/// requested units. Unknown or uncommitted units are omitted. The complete
/// attribution is computed once per `HEAD` and cached in memory.
///
/// # Errors
///
/// Returns a typed command error for an invalid ID or a Git failure.
pub async fn git_unit_attribution(
    app: tauri::AppHandle,
    translation_unit_ids: Vec<String>,
) -> CommandResult<Vec<UnitAttributionDto>> {
    run_blocking(move || {
        let ids = translation_unit_ids
            .iter()
            .map(|id| parse_translation_unit_id(id))
            .collect::<CommandResult<Vec<_>>>()?;
        let attribution = attribution_with_state(&app.state::<DesktopState>())?;
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                attribution
                    .binary_search_by_key(&id, |entry| entry.id)
                    .ok()
                    .map(|index| (&attribution[index]).into())
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Counts committed strings by the translators who translated and reviewed
/// their current text.
///
/// # Errors
///
/// Returns a typed command error when Git fails or history data is invalid.
pub async fn git_contributors(app: tauri::AppHandle) -> CommandResult<Vec<ContributorDto>> {
    run_blocking(move || {
        let attribution = attribution_with_state(&app.state::<DesktopState>())?;
        Ok(summarize_contributors(&attribution)
            .into_iter()
            .map(Into::into)
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Fetches, integrates incoming commits, and pushes local checkpoints.
///
/// Shards are merged per translation unit. Same-unit conflicts are returned
/// in the result without changing the repository; the renderer syncs again
/// with a resolution for each. Integration is rolled back when the resulting
/// workspace does not reload against the active source package.
///
/// # Errors
///
/// Returns a typed command error for uncommitted translations, non-translation
/// conflicts, rejected incoming changes, or a Git/network failure.
pub async fn git_sync(
    app: tauri::AppHandle,
    resolutions: Option<Vec<UnitResolutionDto>>,
) -> CommandResult<GitSyncDto> {
    run_sync(app, move |state| {
        git_sync_with_state(state, resolutions.as_deref().unwrap_or_default())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Lists local and remote-tracking branches.
///
/// # Errors
///
/// Returns a typed command error when Git fails.
pub async fn git_branches(app: tauri::AppHandle) -> CommandResult<Vec<GitBranchDto>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let repository = open_repository(&state)?;
        let content_id = {
            let project = state.lock_project()?;
            let session = project.as_ref().ok_or_else(CommandError::no_project)?;
            session.source().metadata().content_id
        };
        Ok(repository
            .branches()?
            .into_iter()
            .map(|branch| {
                let blocked = (!branch.remote && !branch.current)
                    .then(|| branch_block(&repository, &branch.name, &content_id))
                    .flatten();
                let merged = !branch.remote
                    && !branch.current
                    && repository
                        .is_merged_into_main(&branch.name)
                        .unwrap_or(false);
                GitBranchDto {
                    merged,
                    blocked,
                    ..branch.into()
                }
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Creates a branch at the current commit and switches to it, keeping
/// uncommitted work.
///
/// # Errors
///
/// Returns a typed command error for an invalid or existing name.
pub async fn git_create_branch(app: tauri::AppHandle, name: String) -> CommandResult<()> {
    run_blocking(move || {
        let repository = open_repository(&app.state::<DesktopState>())?;
        Ok(repository.create_branch(&name)?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Switches to a branch and reloads the project; the switch is undone when
/// the branch does not reload against the active source package.
///
/// # Errors
///
/// Returns a typed command error for uncommitted translations, a rejected
/// branch, or a Git failure.
pub async fn git_switch_branch(app: tauri::AppHandle, name: String) -> CommandResult<()> {
    run_sync(app, move |state| {
        let result = with_session_reload(state, |repository, accept| {
            repository.switch_branch(&name, accept)
        })?;
        result.map_err(|error| match error {
            // The branch holds a project this session cannot load; the switch
            // was undone. Say so plainly and keep the details.
            GitError::IncomingRejected { reason } => CommandError::new(
                "gitSwitchIncompatible",
                format!("{name} holds the project in a state this session cannot open, so Aeria stayed on the current branch: {reason}"),
            ),
            other => other.into(),
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Deletes a local branch other than the current one; `force` is required
/// for a branch with commits outside the main branch.
///
/// # Errors
///
/// Returns a typed command error for the current, an unknown, or an unmerged
/// branch without `force`.
pub async fn git_delete_branch(
    app: tauri::AppHandle,
    name: String,
    force: bool,
) -> CommandResult<()> {
    run_blocking(move || {
        open_repository(&app.state::<DesktopState>())?.delete_branch(&name, force)?;
        Ok(())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Sets the main branch contributions are reviewed into, or clears it so it
/// is detected. Writes `aeria-collaboration.json`; nothing is committed.
///
/// # Errors
///
/// Returns a typed command error for an invalid branch name or a Git failure.
pub async fn git_set_main_branch(
    app: tauri::AppHandle,
    main_branch: Option<String>,
) -> CommandResult<CollaborationDto> {
    run_blocking(move || {
        let repository = open_repository(&app.state::<DesktopState>())?;
        repository.set_collaboration(&CollaborationSettings {
            main_branch: main_branch
                .map(|branch| branch.trim().to_owned())
                .filter(|branch| !branch.is_empty()),
        })?;
        Ok(collaboration_dto(&repository)?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns to the main branch after a reviewed contribution was merged and
/// deletes the contribution branch when Git confirms it is merged.
///
/// # Errors
///
/// Returns a typed command error outside the pull-request policy, for
/// uncommitted translations, or for a Git failure.
pub async fn git_finish_contribution(app: tauri::AppHandle) -> CommandResult<GitFinishDto> {
    run_sync(app, move |state| {
        let outcome = with_session_reload(state, |repository, accept| {
            repository.finish_contribution(accept)
        })??;
        Ok(GitFinishDto {
            integration: outcome.integration.into(),
            deleted_branch: outcome.deleted_branch,
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Merges the current contribution branch into the main branch in a
/// repository without remotes, reloading and validating the project.
///
/// # Errors
///
/// Returns a typed command error when the repository has a remote, the
/// merge conflicts, or the merged project is not valid.
pub async fn git_merge_contribution(app: tauri::AppHandle) -> CommandResult<GitFinishDto> {
    run_sync(app, move |state| {
        let outcome = with_session_reload(state, |repository, accept| {
            repository.merge_contribution_locally(accept)
        })??;
        Ok(GitFinishDto {
            integration: outcome.integration.into(),
            deleted_branch: outcome.deleted_branch,
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Clones a translation repository into a new folder named after the
/// repository and returns its path. Without `parent`, the folder is created
/// in the default projects directory. The project is opened afterwards through
/// the ordinary open flow.
///
/// # Errors
///
/// Returns a typed command error for an unsafe URL, a URL that names no
/// folder, an existing destination, or a failed clone.
pub async fn git_clone_repository(
    app: tauri::AppHandle,
    url: String,
    parent: Option<String>,
) -> CommandResult<String> {
    let parent = match parent.filter(|parent| !parent.trim().is_empty()) {
        Some(parent) => PathBuf::from(parent.trim()),
        None => crate::commands::default_projects_directory(&app)?,
    };
    run_sync(app, move |state| {
        let repository = GitRepository::clone_into(url.trim(), parent, state.git())?;
        Ok(repository.root().to_string_lossy().into_owned())
    })
    .await
}

/// Runs a Git operation that fetches, pushes, commits, or changes the working
/// tree, and that an application update therefore waits for.
async fn run_sync<T, F>(app: tauri::AppHandle, operation: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce(&DesktopState) -> CommandResult<T> + Send + 'static,
{
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let _sync = state.begin_activity(Activity::Sync);
        operation(&state)
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use aeria_core::SourceBinding;

    use super::*;
    use crate::commands::{
        initialize_project_with_state, open_project_with_state, set_translation_target_with_state,
    };
    use crate::dto::SourceBindingDto;

    fn isolated_git(sandbox: &Path) -> GitExecutable {
        let global = sandbox.join("gitconfig");
        fs::write(&global, "").expect("global config");
        let program = std::env::var_os("AERIA_GIT_PATH")
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "git".into());
        GitExecutable::at(program)
            .with_env("GIT_CONFIG_GLOBAL", global)
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
    }

    #[test]
    fn branches_holding_another_project_state_are_blocked() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let git = isolated_git(sandbox.path());
        let root = sandbox.path().join("project");
        fs::create_dir_all(root.join(".aeria")).expect("project");
        let repository = GitRepository::init(&root, git).expect("init");
        repository
            .set_identity("Ada", None, false)
            .expect("identity");
        let commit = |manifest: &str, message: &str| {
            fs::write(root.join(".aeria/manifest.json"), manifest).expect("manifest");
            repository.checkpoint(Some(message)).expect("commit");
        };
        commit(r#"{"formatVersion":1,"contentId":"sha256:old"}"#, "old");
        let old = repository.status().expect("status").branch.expect("branch");
        let current = r#"{"formatVersion":2,"contentId":"sha256:new"}"#;
        commit(current, "migrated");
        let migrated = repository.status().expect("status").branch.expect("branch");
        assert_ne!(
            old, migrated,
            "the second checkpoint started a contribution branch"
        );

        assert_eq!(
            branch_block(&repository, &old, "sha256:new"),
            Some(BranchBlockDto::OlderFormat)
        );
        assert_eq!(branch_block(&repository, &migrated, "sha256:new"), None);
        assert_eq!(
            branch_block(&repository, &migrated, "sha256:other"),
            Some(BranchBlockDto::OtherSource)
        );
    }

    fn binding() -> SourceBindingDto {
        SourceBindingDto {
            sheet_name: "Synthetic".to_owned(),
            row_id: 42,
            subrow_id: 0,
            column_index: 0,
        }
    }

    #[test]
    fn sync_reloads_the_active_session_with_incoming_translations() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let git = isolated_git(sandbox.path());
        let source = sandbox.path().join("source.hsp");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../crates/aeria-hsp/tests/fixtures/synthetic.hsp"),
            &source,
        )
        .expect("source package");
        let remote = sandbox.path().join("remote.git");
        let status = std::process::Command::new(
            std::env::var_os("AERIA_GIT_PATH")
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| "git".into()),
        )
        .args(["init", "--quiet", "--bare", "--initial-branch=main"])
        .arg(&remote)
        .status()
        .expect("create remote");
        assert!(status.success());
        let remote_url = remote.to_string_lossy().into_owned();

        // Ada creates the project and publishes it.
        let ada = DesktopState::new();
        ada.set_git(git.clone());
        let ada_root = sandbox.path().join("ada");
        fs::create_dir_all(&ada_root).expect("ada root");
        initialize_project_with_state(
            &ada,
            ada_root.to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            sandbox.path().join("cache-ada"),
            "fr".to_owned(),
        )
        .expect("initialize");
        let ada_repository = GitRepository::init(&ada_root, git.clone()).expect("init");
        ada_repository
            .set_identity("Ada", None, false)
            .expect("identity");
        ada_repository
            .set_remote("origin", &remote_url)
            .expect("remote");
        git_checkpoint_with_state(&ada, None).expect("initial checkpoint");
        ada_repository.push().expect("publish");

        // Grace clones it and opens the same source.
        let grace_root = sandbox.path().join("grace");
        let grace_repository =
            GitRepository::clone_from(&remote_url, &grace_root, git.clone()).expect("clone");
        grace_repository
            .set_identity("Grace", None, false)
            .expect("identity");
        let grace = DesktopState::new();
        grace.set_git(git);
        open_project_with_state(
            &grace,
            grace_root.to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            sandbox.path().join("cache-grace"),
        )
        .expect("open clone");

        set_translation_target_with_state(&ada, binding(), "Bonjour").expect("translate");
        // The checkpoint on main starts a contribution branch; Ada publishes it.
        let outcome = git_checkpoint_with_state(&ada, None).expect("checkpoint");
        assert!(outcome.branch_created.is_some());
        ada_repository.push().expect("push contribution");
        // The hosting service merges the pull request into main.
        let merged = std::process::Command::new(
            std::env::var_os("AERIA_GIT_PATH")
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| "git".into()),
        )
        .current_dir(&ada_root)
        .args(["push", "--quiet", "origin", "HEAD:refs/heads/main"])
        .status()
        .expect("merge pull request");
        assert!(merged.success());

        let result = git_sync_with_state(&grace, &[]).expect("sync");
        assert!(result.workspace_changed);
        assert!(result.conflicts.is_empty());
        let project = grace.lock_project().expect("lock");
        let unit = project
            .as_ref()
            .expect("project")
            .workspace()
            .unit_by_source_binding(&SourceBinding::from(binding()))
            .expect("incoming unit")
            .clone();
        assert_eq!(unit.target_macro(), "Bonjour");
    }
}
