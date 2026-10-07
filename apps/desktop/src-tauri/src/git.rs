//! Desktop adapter for Git collaboration commands.
//!
//! Git operations run against the active project's repository root. Commands
//! that change the working tree or read it while the editor could write
//! (checkpoint, integration, branch switches) hold the project lock; network
//! operations do not.

use std::collections::BTreeMap;
use std::path::PathBuf;

use aeria_git::{
    BranchInfo, CheckpointOutcome, CommitSummary, ConfigScope, ConflictResolution, EntryChange,
    EntryChangeKind, EntryConflict, EntryHistory, EntryRevision, EntryState, FileChangeKind,
    FileStatus, GitError, GitExecutable, GitOrigin, GitRepository, IntegrateOutcome, RemoteInfo,
    RepositoryStatus, TranslatorIdentity, summarize_changes,
};
use aeria_po::Session;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::commands::run_blocking;
use crate::dto::SourceBindingDto;
use crate::error::CommandError;
use crate::file_changes::{self, FileChangeDto, WorkingChangesDto};
use crate::project_changes;
use crate::state::{Activity, DesktopState};

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

impl From<FileChangeKind> for GitFileKindDto {
    fn from(kind: FileChangeKind) -> Self {
        match kind {
            FileChangeKind::Added => Self::Added,
            FileChangeKind::Modified => Self::Modified,
            FileChangeKind::Deleted => Self::Deleted,
            FileChangeKind::Renamed => Self::Renamed,
            FileChangeKind::Copied => Self::Copied,
            FileChangeKind::TypeChanged => Self::TypeChanged,
            FileChangeKind::Untracked => Self::Untracked,
            FileChangeKind::Conflicted => Self::Conflicted,
        }
    }
}

impl From<FileStatus> for GitFileDto {
    fn from(file: FileStatus) -> Self {
        Self {
            translation_data: file.is_translation_data(),
            kind: file.kind.into(),
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
    /// The commit is on no remote yet: a push would publish it. Set only in
    /// history listings.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unpublished: bool,
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
            unpublished: false,
        }
    }
}

/// What a person can change about a string at one point in time.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryVersionDto {
    pub target_macro: String,
    pub fuzzy: bool,
    pub translator_note: Option<String>,
    /// A person reviewed the translation as it is.
    pub reviewed: bool,
}

impl From<&EntryState> for EntryVersionDto {
    fn from(state: &EntryState) -> Self {
        Self {
            target_macro: state.translation.clone(),
            fuzzy: state.fuzzy,
            translator_note: state.note.clone(),
            reviewed: state.reviewed,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryChangeKindDto {
    Translated,
    Changed,
    Cleared,
    Marked,
}

impl From<EntryChangeKind> for EntryChangeKindDto {
    fn from(kind: EntryChangeKind) -> Self {
        match kind {
            EntryChangeKind::Translated => Self::Translated,
            EntryChangeKind::Changed => Self::Changed,
            EntryChangeKind::Cleared => Self::Cleared,
            EntryChangeKind::Marked => Self::Marked,
        }
    }
}

/// The coordinate of an entry's string in the open project's game.
fn coordinate(session: Option<&Session>, context: &str) -> Option<SourceBindingDto> {
    let (sheet_name, row_id, subrow_id, column_index) = session?.coordinate_of(context)?;
    Some(SourceBindingDto {
        sheet_name,
        row_id,
        subrow_id,
        column_index,
    })
}

/// A change of one string.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryChangeDto {
    /// The entry's `msgctxt`.
    pub context: String,
    /// The PO file, relative to the project root.
    pub path: String,
    /// Where the string is in the game; `None` when the game has no such
    /// string.
    pub source_binding: Option<SourceBindingDto>,
    pub source_macro: String,
    pub kind: EntryChangeKindDto,
    pub before: EntryVersionDto,
    pub after: EntryVersionDto,
}

impl EntryChangeDto {
    fn new(change: &EntryChange, session: Option<&Session>) -> Self {
        Self {
            context: change.context.clone(),
            path: change.path.clone(),
            source_binding: coordinate(session, &change.context),
            source_macro: change.source.clone(),
            kind: change.kind.into(),
            before: (&change.before).into(),
            after: (&change.after).into(),
        }
    }
}

fn changes_dto(changes: &[EntryChange], session: Option<&Session>) -> Vec<EntryChangeDto> {
    changes
        .iter()
        .map(|change| EntryChangeDto::new(change, session))
        .collect()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryRevisionDto {
    pub commit: GitCommitDto,
    pub kind: EntryChangeKindDto,
    pub before: EntryVersionDto,
    pub after: EntryVersionDto,
}

impl From<EntryRevision> for EntryRevisionDto {
    fn from(revision: EntryRevision) -> Self {
        Self {
            kind: revision.kind.into(),
            before: (&revision.before).into(),
            after: (&revision.after).into(),
            commit: revision.commit.into(),
        }
    }
}

/// The history of one string.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringHistoryDto {
    pub pending: Option<EntryChangeDto>,
    pub revisions: Vec<EntryRevisionDto>,
    pub truncated: bool,
}

impl StringHistoryDto {
    fn new(history: EntryHistory, session: Option<&Session>) -> Self {
        Self {
            pending: history
                .pending
                .as_ref()
                .map(|change| EntryChangeDto::new(change, session)),
            revisions: history.revisions.into_iter().map(Into::into).collect(),
            truncated: history.truncated,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitChangesDto {
    pub commit: GitCommitDto,
    pub changes: Vec<EntryChangeDto>,
    /// Every file the commit changed, with its string count or readable
    /// change; empty for the result of a checkpoint.
    pub files: Vec<FileChangeDto>,
    /// No remote-tracking branch has the commit.
    pub local: bool,
    /// The name of the remote the branch syncs with.
    pub remote: Option<String>,
    /// The commit's page on the hosting service of the sync remote.
    pub web_url: Option<String>,
}

impl GitCommitChangesDto {
    fn new(outcome: CheckpointOutcome, session: Option<&Session>) -> Self {
        Self {
            changes: changes_dto(&outcome.changes, session),
            commit: outcome.commit.into(),
            files: Vec::new(),
            local: true,
            remote: None,
            web_url: None,
        }
    }
}

/// How many string changes each file of `changes` has.
fn strings_per_file<'a>(
    changes: impl IntoIterator<Item = &'a EntryChange>,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for change in changes {
        *counts.entry(change.path.clone()).or_default() += 1;
    }
    counts
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

/// One string changed differently here and on the remote.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryConflictDto {
    pub context: String,
    pub path: String,
    pub source_binding: Option<SourceBindingDto>,
    pub source_macro: String,
    pub base: EntryVersionDto,
    pub ours: EntryVersionDto,
    pub theirs: EntryVersionDto,
}

impl EntryConflictDto {
    fn new(conflict: &EntryConflict, session: Option<&Session>) -> Self {
        Self {
            context: conflict.context.clone(),
            path: conflict.path.clone(),
            source_binding: coordinate(session, &conflict.context),
            source_macro: conflict.source.clone(),
            base: (&conflict.base).into(),
            ours: (&conflict.ours).into(),
            theirs: (&conflict.theirs).into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictResolutionDto {
    Ours,
    Theirs,
}

/// Which side of a conflicting string to keep.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryResolutionDto {
    pub context: String,
    pub resolution: ConflictResolutionDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPullDto {
    pub integration: GitIntegrationDto,
    /// Whether the renderer must reload translation data.
    pub workspace_changed: bool,
    /// Strings changed differently on both sides. When non-empty nothing was
    /// integrated; pull again with a resolution for every conflict.
    pub conflicts: Vec<EntryConflictDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchDto {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: Option<String>,
    /// Commits that deleting the branch would lose: commits no other branch,
    /// remote-tracking branch, or tag contains. For a remote branch, deleting
    /// it on the remote.
    pub lost_commits: u32,
    /// For a local branch with an upstream on a remote, the commits deleting
    /// both would lose.
    pub lost_with_upstream: Option<u32>,
    /// Why the open project cannot switch to this branch, or `None`.
    pub blocked: Option<BranchBlockDto>,
}

/// Why a branch holds a project the open session cannot load as is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchBlockDto {
    /// The branch has no Aeria project.
    NoProject,
    /// The branch stores a project format this version does not read.
    OlderFormat,
    /// The branch's files are for another game version.
    OtherSource,
}

/// Compares the `aeria.json` a branch stores with the open session: the
/// same format and the same game version. Unreadable settings are left to
/// the switch itself.
fn branch_block(
    repository: &GitRepository,
    branch: &str,
    game_version: &str,
) -> Option<BranchBlockDto> {
    let revision = repository.branch_head(branch).ok().flatten()?;
    let Ok(settings) = repository.file_at(&revision, aeria_po::SETTINGS_FILE) else {
        return None;
    };
    let Some(settings) = settings else {
        return Some(BranchBlockDto::NoProject);
    };
    let value: serde_json::Value = serde_json::from_slice(&settings).ok()?;
    let format = value.get("format").and_then(serde_json::Value::as_str);
    if !format.is_some_and(|format| aeria_po::READ_FORMATS.contains(&format)) {
        return Some(BranchBlockDto::OlderFormat);
    }
    (value.get("gameVersion").and_then(serde_json::Value::as_str) != Some(game_version))
        .then_some(BranchBlockDto::OtherSource)
}

impl From<BranchInfo> for GitBranchDto {
    fn from(branch: BranchInfo) -> Self {
        Self {
            name: branch.name,
            remote: branch.remote,
            current: branch.current,
            upstream: branch.upstream,
            lost_commits: 0,
            lost_with_upstream: None,
            blocked: None,
        }
    }
}

fn project_root(state: &DesktopState) -> CommandResult<PathBuf> {
    Ok(state.session()?.root().to_owned())
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

/// Runs a Git operation that can change the working tree while the
/// session's writes are held. `accept` checks the result: the project must
/// still be for the open game version. The outer result reports state
/// errors, the inner one the Git operation.
fn with_session_reload<T>(
    state: &DesktopState,
    operation: impl FnOnce(
        &GitRepository,
        &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<T, GitError>,
) -> CommandResult<Result<T, GitError>> {
    let session = state.session()?;
    let _writes = session.hold_writes();
    let repository = GitRepository::open(session.root(), state.git())?;
    let game_version = session.source().version().to_string();
    let mut accept = || {
        let settings =
            aeria_po::read_settings(session.root()).map_err(|error| error.to_string())?;
        if settings.game_version == game_version {
            Ok(())
        } else {
            Err(format!(
                "its files are for game version {}, and the open game is {game_version}",
                settings.game_version
            ))
        }
    };
    let result = operation(&repository, &mut accept);
    // The working tree may have changed: views of the files read them again.
    session.touch();
    Ok(result)
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
        });
    };
    Ok(GitOverviewDto {
        runtime,
        repository: Some(repository.status()?.into()),
        identity: Some(repository.identity()?.into()),
        remotes: repository.remotes()?.into_iter().map(Into::into).collect(),
    })
}

pub(crate) fn git_checkpoint_with_state(
    state: &DesktopState,
    message: Option<&str>,
) -> CommandResult<GitCommitChangesDto> {
    let session = state.session()?;
    let _writes = session.hold_writes();
    let repository = GitRepository::open(session.root(), state.git())?;
    let project_files = project_changes::pending(&repository, session.root())?;
    let message = if let Some(message) = message.map(str::trim).filter(|text| !text.is_empty()) {
        Some(message.to_owned())
    } else {
        let changes = repository.pending_changes()?;
        let translations = (!changes.is_empty()).then(|| summarize_changes(&changes));
        project_changes::checkpoint_message(translations.as_deref(), &project_files)
    };
    let outcome =
        GitCommitChangesDto::new(repository.checkpoint(message.as_deref())?, Some(&session));
    Ok(outcome)
}

/// Fetches and integrates the upstream, with the per-string merge of PO files
/// and validation of the resulting project.
pub(crate) fn pull_with_state(
    state: &DesktopState,
    resolutions: &[EntryResolutionDto],
) -> CommandResult<GitPullDto> {
    let resolutions: BTreeMap<String, ConflictResolution> = resolutions
        .iter()
        .map(|entry| {
            let resolution = match entry.resolution {
                ConflictResolutionDto::Ours => ConflictResolution::Ours,
                ConflictResolutionDto::Theirs => ConflictResolution::Theirs,
            };
            (entry.context.clone(), resolution)
        })
        .collect();

    let repository = open_repository(state)?;
    repository.fetch()?;
    let integration = with_session_reload(state, |repository, accept| {
        repository.integrate(&resolutions, accept)
    })?;
    let integration = match integration {
        Ok(integration) => integration,
        // Nothing was integrated; the repository is unchanged.
        Err(GitError::TranslationConflicts { conflicts }) => {
            let session = state.session().ok();
            return Ok(GitPullDto {
                integration: GitIntegrationDto::UpToDate,
                workspace_changed: false,
                conflicts: conflicts
                    .iter()
                    .map(|conflict| EntryConflictDto::new(conflict, session.as_deref()))
                    .collect(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    Ok(GitPullDto {
        integration: integration.into(),
        workspace_changed: integration.changed_working_tree(),
        conflicts: Vec::new(),
    })
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the Git runtime, repository status, translator identity, and
/// remotes for the active project.
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

/// The most uncommitted string changes listed at once; a project without a
/// first commit can have hundreds of thousands.
const PENDING_LISTED: usize = 500;

/// The uncommitted string changes: how many there are, and the first of
/// them in file order.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChangesDto {
    pub total: usize,
    pub changes: Vec<EntryChangeDto>,
}

fn pending_changes_with_state(state: &DesktopState) -> CommandResult<PendingChangesDto> {
    let session = state.session()?;
    let repository = GitRepository::open(session.root(), state.git())?;
    let files = repository.pending_files(&mut state.pending_cache())?;
    let total = files.iter().map(|(_, changes)| changes.len()).sum();
    let listed: Vec<EntryChange> = files
        .iter()
        .flat_map(|(_, changes)| changes.iter())
        .take(PENDING_LISTED)
        .cloned()
        .collect();
    Ok(PendingChangesDto {
        total,
        changes: changes_dto(&listed, Some(&session)),
    })
}

/// Whether `path` (`po/...`) is a file of the sheet whose files are named
/// after `base`: `po/{base}.po`, or `po/{base}/{start}.po` for a sheet split
/// by row range.
fn is_sheet_file(path: &str, base: &str) -> bool {
    let Some(rest) = path
        .strip_prefix(aeria_git::PO_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|rest| rest.strip_prefix(base))
    else {
        return false;
    };
    rest == ".po"
        || rest
            .strip_prefix('/')
            .and_then(|file| file.strip_suffix(".po"))
            .is_some_and(|start| {
                !start.is_empty() && start.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn pending_sheet_changes_with_state(
    state: &DesktopState,
    sheet_name: &str,
) -> CommandResult<Vec<EntryChangeDto>> {
    let session = state.session()?;
    let repository = GitRepository::open(session.root(), state.git())?;
    let base = session.sheet_base(sheet_name);
    let files = repository.pending_files(&mut state.pending_cache())?;
    let changes: Vec<EntryChange> = files
        .iter()
        .filter(|(path, _)| is_sheet_file(path, &base))
        .flat_map(|(_, changes)| changes.iter().cloned())
        .collect();
    Ok(changes_dto(&changes, Some(&session)))
}

#[tauri::command(rename_all = "camelCase")]
/// Returns how many strings have uncommitted changes, and the first
/// [`PENDING_LISTED`] of those changes.
///
/// # Errors
///
/// Returns a typed command error when no project is open or Git fails.
pub async fn git_pending_changes(app: tauri::AppHandle) -> CommandResult<PendingChangesDto> {
    run_blocking(move || pending_changes_with_state(&app.state::<DesktopState>())).await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns every uncommitted string change of one sheet.
///
/// # Errors
///
/// Returns a typed command error when no project is open or Git fails.
pub async fn git_pending_sheet_changes(
    app: tauri::AppHandle,
    sheet_name: String,
) -> CommandResult<Vec<EntryChangeDto>> {
    run_blocking(move || {
        pending_sheet_changes_with_state(&app.state::<DesktopState>(), &sheet_name)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Commits as Git tools do: what is staged when anything is, otherwise
/// every change of the translations and project files (a checkpoint). With
/// `amend`, the last commit is replaced instead, while no remote has it. A
/// blank message is replaced by a generated summary (an amend keeps the
/// commit's message).
///
/// # Errors
///
/// Returns a typed command error when the name is missing, there is nothing
/// to commit, the last commit is already pushed (`amend`), or Git fails.
pub async fn git_commit(
    app: tauri::AppHandle,
    message: Option<String>,
    amend: bool,
) -> CommandResult<GitCommitChangesDto> {
    run_sync(app, move |state| {
        let session = state.session()?;
        let repository = GitRepository::open(session.root(), state.git())?;
        if amend {
            let _writes = session.hold_writes();
            return Ok(GitCommitChangesDto::new(
                repository.amend(message.as_deref())?,
                Some(&session),
            ));
        }
        if repository.has_staged_changes()? {
            return Ok(GitCommitChangesDto::new(
                repository.commit_staged(message.as_deref())?,
                Some(&session),
            ));
        }
        git_checkpoint_with_state(state, message.as_deref())
    })
    .await
}

/// Paths of one stage, unstage, or discard request, at most this many.
const MAX_PATHS: usize = 100_000;

fn bounded_paths(paths: &[String]) -> CommandResult<()> {
    if paths.len() > MAX_PATHS {
        return Err(CommandError::new(
            "gitInvalidInput",
            format!("at most {MAX_PATHS} files at once"),
        ));
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
/// Stages files (`git add --all`).
///
/// # Errors
///
/// Returns a typed command error for an invalid path or a Git failure.
pub async fn git_stage(app: tauri::AppHandle, paths: Vec<String>) -> CommandResult<()> {
    bounded_paths(&paths)?;
    run_sync(app, move |state| Ok(open_repository(state)?.stage(&paths)?)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Takes files out of the index, keeping their working-tree content.
///
/// # Errors
///
/// Returns a typed command error for an invalid path or a Git failure.
pub async fn git_unstage(app: tauri::AppHandle, paths: Vec<String>) -> CommandResult<()> {
    bounded_paths(&paths)?;
    run_sync(app, move |state| {
        Ok(open_repository(state)?.unstage(&paths)?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Discards the working-tree changes of files: tracked files get back their
/// staged or committed content, untracked files are deleted. The editor's
/// writes wait meanwhile, and its views read the files again.
///
/// # Errors
///
/// Returns a typed command error for an invalid path, a conflicted file, or
/// a Git or file system failure.
pub async fn git_discard(app: tauri::AppHandle, paths: Vec<String>) -> CommandResult<()> {
    bounded_paths(&paths)?;
    run_sync(app, move |state| {
        let session = state.session()?;
        let _writes = session.hold_writes();
        let result = GitRepository::open(session.root(), state.git())?.discard(&paths);
        session.touch();
        Ok(result?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Undoes the last commit while no remote has it: its changes go back to the
/// index. Returns the commit undone.
///
/// # Errors
///
/// Returns `gitCommitPublished` for a pushed commit, or another typed Git
/// error.
pub async fn git_undo_last_commit(app: tauri::AppHandle) -> CommandResult<GitCommitDto> {
    run_sync(app, |state| {
        Ok(open_repository(state)?.undo_last_commit()?.into())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Reverts a commit with a new commit; PO files are joined per string, and
/// the result must be a project for the open game version.
///
/// # Errors
///
/// Returns a typed command error when translations are uncommitted, a
/// string or file conflicts, the result is not a valid project, or Git
/// fails; the branch is then unchanged.
pub async fn git_revert(app: tauri::AppHandle, commit_id: String) -> CommandResult<GitCommitDto> {
    run_sync(app, move |state| {
        let result = with_session_reload(state, |repository, accept| {
            repository.revert(&commit_id, accept)
        })?;
        Ok(result?.into())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Creates a branch at a commit and switches to it.
///
/// # Errors
///
/// Returns a typed command error when translations are uncommitted, the
/// name is invalid, or the commit holds a project this session cannot open.
pub async fn git_create_branch_at(
    app: tauri::AppHandle,
    name: String,
    commit_id: String,
) -> CommandResult<()> {
    run_sync(app, move |state| {
        let result = with_session_reload(state, |repository, accept| {
            repository.create_branch_at(&name, &commit_id, accept)
        })?;
        result.map_err(|error| match error {
            GitError::IncomingRejected { reason } => CommandError::new(
                "gitSwitchIncompatible",
                format!("the commit holds the project in a state this session cannot open, so Aeria stayed on the current branch: {reason}"),
            ),
            other => other.into(),
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the uncommitted files: what the index holds against `HEAD`, and
/// the working tree's changes against the index, PO files with how many
/// strings changed and project files with their readable change.
///
/// # Errors
///
/// Returns a typed command error when no project is open or Git fails.
pub async fn git_changed_files(app: tauri::AppHandle) -> CommandResult<WorkingChangesDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let repository = open_repository(&state)?;
        Ok(file_changes::working_changes(
            &repository,
            &mut state.pending_cache(),
        )?)
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the string changes of one PO file: staged ones against `HEAD`,
/// or the working tree's against the index.
///
/// # Errors
///
/// Returns a typed command error when no project is open or Git fails.
pub async fn git_pending_file_changes(
    app: tauri::AppHandle,
    path: String,
    staged: bool,
) -> CommandResult<Vec<EntryChangeDto>> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let session = state.session()?;
        let repository = GitRepository::open(session.root(), state.git())?;
        let changes = file_changes::file_strings(&repository, &path, staged)?;
        Ok(changes_dto(&changes, Some(&session)))
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
    query: Option<String>,
    path: Option<String>,
) -> CommandResult<Vec<GitCommitDto>> {
    run_blocking(move || {
        let limit = bounded_limit(limit)?;
        let repository = open_repository(&app.state::<DesktopState>())?;
        let skip = usize::try_from(skip).unwrap_or(usize::MAX);
        let query = query.as_deref().map(str::trim).unwrap_or_default();
        let commits = if query.is_empty() && path.is_none() {
            repository.log(skip, limit)?
        } else {
            repository.search_log(skip, limit, query, path.as_deref())?
        };
        let unpublished = repository.unpublished_commits()?;
        Ok(commits
            .into_iter()
            .map(|commit| GitCommitDto {
                unpublished: unpublished.contains(&commit.id),
                ..commit.into()
            })
            .collect())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Returns the string changes introduced by one commit.
///
/// # Errors
///
/// Returns a typed command error for an invalid commit ID or a Git failure.
pub async fn git_commit_changes(
    app: tauri::AppHandle,
    commit_id: String,
) -> CommandResult<GitCommitChangesDto> {
    run_sync(app, move |state| {
        let session = state.session()?;
        let repository = GitRepository::open(session.root(), state.git())?;
        let (commit, changes) = repository.commit_changes(&commit_id)?;
        let files = file_changes::changed_files(
            repository.commit_files(&commit.id)?,
            &strings_per_file(&changes),
            project_changes::of_commit(&repository, &commit.id)?,
        );
        let web_url = commit_web_url(&repository, &commit.id)?;
        Ok(GitCommitChangesDto {
            files,
            local: !repository.is_published(&commit.id)?,
            remote: sync_remote(&repository)?.map(|remote| remote.name),
            web_url,
            commit: commit.into(),
            changes: changes_dto(&changes, Some(&session)),
        })
    })
    .await
}

/// Commits of a file read to find the history of one of its strings.
const HISTORY_SCAN: usize = 200;

#[tauri::command(rename_all = "camelCase")]
/// Returns the history of one string: its uncommitted change and the commits
/// that changed it, newest first.
///
/// # Errors
///
/// Returns a typed command error for an invalid limit, a string that is not
/// in the project, or a Git failure.
pub async fn git_string_history(
    app: tauri::AppHandle,
    source_binding: SourceBindingDto,
    limit: u32,
) -> CommandResult<StringHistoryDto> {
    run_blocking(move || {
        let limit = bounded_limit(limit)?;
        let state = app.state::<DesktopState>();
        let session = state.session()?;
        let (path, context, _) = session.locate(
            &source_binding.sheet_name,
            source_binding.row_id,
            source_binding.subrow_id,
            source_binding.column_index,
        )?;
        let repository = GitRepository::open(session.root(), state.git())?;
        let history = repository.entry_history(
            &format!("{}/{path}", aeria_po::PO_DIR),
            &context,
            limit,
            HISTORY_SCAN,
        )?;
        Ok(StringHistoryDto::new(history, Some(&session)))
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
        let game_version = state.session()?.source().version().to_string();
        let remotes = repository.remotes()?;
        Ok(repository
            .branches()?
            .into_iter()
            .map(|branch| {
                let blocked = (!branch.remote && !branch.current)
                    .then(|| branch_block(&repository, &branch.name, &game_version))
                    .flatten();
                let (lost_commits, lost_with_upstream) = if branch.remote {
                    (
                        repository.lost_remote_commits(&branch.name).unwrap_or(0),
                        None,
                    )
                } else {
                    let remote_upstream = branch.upstream.as_deref().is_some_and(|upstream| {
                        remotes
                            .iter()
                            .any(|remote| upstream.starts_with(&format!("{}/", remote.name)))
                    });
                    (
                        repository.lost_commits(&branch.name, false).unwrap_or(0),
                        remote_upstream
                            .then(|| repository.lost_commits(&branch.name, true).ok())
                            .flatten(),
                    )
                };
                GitBranchDto {
                    lost_commits,
                    lost_with_upstream,
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
/// the branch does not reload against the open game source.
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
/// Deletes a local branch other than the current one, and with
/// `with_upstream` its upstream branch on the remote too. `force` is
/// required when the deletion would lose commits that exist on no other
/// branch.
///
/// # Errors
///
/// Returns a typed command error for the current or an unknown branch, for
/// lost commits without `force`, or when the remote refuses the deletion.
pub async fn git_delete_branch(
    app: tauri::AppHandle,
    name: String,
    with_upstream: bool,
    force: bool,
) -> CommandResult<()> {
    run_blocking(move || {
        open_repository(&app.state::<DesktopState>())?.delete_branch(
            &name,
            with_upstream,
            force,
        )?;
        Ok(())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Deletes a branch on its remote, given by its remote-tracking name
/// (`origin/feature`). `force` is required when the deletion would lose
/// commits that exist on no other branch.
///
/// # Errors
///
/// Returns a typed command error for an unknown branch, for lost commits
/// without `force`, or when the remote refuses the deletion.
pub async fn git_delete_remote_branch(
    app: tauri::AppHandle,
    name: String,
    force: bool,
) -> CommandResult<()> {
    run_blocking(move || {
        open_repository(&app.state::<DesktopState>())?.delete_remote_branch(&name, force)?;
        Ok(())
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

#[tauri::command(rename_all = "camelCase")]
/// Fetches the current branch's remote without changing the branch, so the
/// dock shows how far it is behind.
///
/// # Errors
///
/// Returns a typed Git error, for example without a remote or network.
pub async fn git_fetch(app: tauri::AppHandle) -> CommandResult<()> {
    run_sync(app, |state| Ok(open_repository(state)?.fetch()?)).await
}

#[tauri::command(rename_all = "camelCase")]
/// Fetches and integrates the upstream, without pushing. PO files both sides
/// changed are joined per string; strings changed differently on both sides
/// are returned without changing the repository, and the renderer pulls
/// again with a resolution for each. Integration is rolled back when the
/// result is for another game version.
///
/// # Errors
///
/// Returns a typed Git error, for example with uncommitted translations.
pub async fn git_pull(
    app: tauri::AppHandle,
    resolutions: Option<Vec<EntryResolutionDto>>,
) -> CommandResult<GitPullDto> {
    run_sync(app, move |state| {
        pull_with_state(state, resolutions.as_deref().unwrap_or_default())
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Pushes the current branch's commits, publishing it when it has no
/// upstream. Refuses while the upstream has commits this branch lacks, so a
/// push never needs to be forced. Returns whether anything was pushed.
///
/// # Errors
///
/// Returns `gitPushBehind` when the branch must pull first, or a typed Git
/// error.
pub async fn git_push(app: tauri::AppHandle) -> CommandResult<bool> {
    run_sync(app, |state| {
        let repository = open_repository(state)?;
        let status = repository.status()?;
        if status.upstream.is_some() && status.behind > 0 {
            return Err(CommandError::new(
                "gitPushBehind",
                format!(
                    "the upstream has {} commits this branch lacks; pull first",
                    status.behind
                ),
            ));
        }
        Ok(repository.push()?)
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

    use super::*;
    use crate::commands::{
        initialize_with_game, open_with_game, page_translation_rows_with_state,
        set_translation_target_with_state,
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
        fs::create_dir_all(&root).expect("project");
        let repository = GitRepository::init(&root, git).expect("init");
        repository
            .set_identity("Ada", None, false)
            .expect("identity");
        let commit = |settings: &str, message: &str| {
            fs::write(root.join("aeria.json"), settings).expect("settings");
            repository.checkpoint(Some(message)).expect("commit");
        };
        commit(r#"{"formatVersion":3}"#, "old");
        let old = repository.status().expect("status").branch.expect("branch");
        let migrated = "migrated";
        repository.create_branch(migrated).expect("branch");
        let current = r#"{"format":"aeria-po/1","gameVersion":"2026.10.01.0000.0000"}"#;
        commit(current, "current");

        assert_eq!(
            branch_block(&repository, &old, "2026.10.01.0000.0000"),
            Some(BranchBlockDto::OlderFormat)
        );
        assert_eq!(
            branch_block(&repository, migrated, "2026.10.01.0000.0000"),
            None
        );
        assert_eq!(
            branch_block(&repository, migrated, "2026.11.01.0000.0000"),
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
    #[allow(clippy::too_many_lines)] // one scenario
    fn pull_and_sync_bring_incoming_translations_into_the_open_project() {
        let sandbox = tempfile::tempdir().expect("sandbox");
        let git = isolated_git(sandbox.path());
        let game = crate::test_support::test_game();
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
        initialize_with_game(
            &ada,
            &ada_root,
            crate::test_support::open(game.path()),
            &sandbox.path().join("cache-ada"),
            "fr",
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

        // Grace clones it and opens it with the same game.
        let grace_root = sandbox.path().join("grace");
        let grace_repository =
            GitRepository::clone_from(&remote_url, &grace_root, git.clone()).expect("clone");
        grace_repository
            .set_identity("Grace", None, false)
            .expect("identity");
        let grace = DesktopState::new();
        grace.set_git(git);
        let opened = open_with_game(
            &grace,
            &grace_root,
            crate::test_support::open(game.path()),
            &sandbox.path().join("cache-grace"),
        )
        .expect("open clone");
        assert!(matches!(
            opened,
            crate::commands::GameOpenOutcome::Opened { .. }
        ));

        set_translation_target_with_state(&ada, &binding(), "Bonjour").expect("translate");
        let pending = git_pending_changes_of(&ada);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source_binding.as_ref(), Some(&binding()));
        // Ada commits on main and pushes it.
        let outcome = git_checkpoint_with_state(&ada, None).expect("checkpoint");
        assert_eq!(outcome.changes.len(), 1);
        assert!(ada_repository.push().expect("push main"));

        let result = pull_with_state(&grace, &[]).expect("pull");
        assert!(result.workspace_changed);
        assert!(result.conflicts.is_empty());
        let again = pull_with_state(&grace, &[]).expect("pull again");
        assert!(matches!(again.integration, GitIntegrationDto::UpToDate));
        let page = page_translation_rows_with_state(&grace, "Synthetic", None, 10).expect("page");
        let translation = page.rows[1].cells[0]
            .translation
            .as_ref()
            .expect("incoming");
        assert_eq!(translation.target_macro, "Bonjour");
    }

    fn git_pending_changes_of(state: &DesktopState) -> Vec<EntryChangeDto> {
        let pending = pending_changes_with_state(state).expect("pending");
        let sheet = pending_sheet_changes_with_state(state, "Synthetic").expect("sheet");
        assert_eq!(pending.total, pending.changes.len());
        assert_eq!(sheet.len(), pending.total);
        pending.changes
    }

    #[test]
    fn sheet_files_are_matched_by_base() {
        assert!(is_sheet_file("po/Addon.po", "Addon"));
        assert!(is_sheet_file("po/Item/2000.po", "Item"));
        assert!(is_sheet_file("po/quest/001/Q_1.po", "quest/001/Q_1"));
        assert!(!is_sheet_file("po/AddonTransient.po", "Addon"));
        assert!(!is_sheet_file("po/Item/x/2000.po", "Item"));
        assert!(!is_sheet_file("po/Item/.po", "Item"));
        assert!(!is_sheet_file("Addon.po", "Addon"));
    }
}

/// The web page of a commit on the hosting service of the remote the
/// current branch syncs with (its upstream's remote, else `origin`, else the
/// only remote): `https://<host>/<path>/commit/<id>`, with GitLab's
/// `/-/commit/` and Bitbucket's `/commits/`. `None` for a local path or a
/// URL that names no host and repository.
fn commit_web_url(repository: &GitRepository, commit: &str) -> CommandResult<Option<String>> {
    Ok(sync_remote(repository)?.and_then(|remote| web_url(&remote.url, commit)))
}

/// The remote the current branch syncs with: its upstream's remote, else
/// `origin`, else the only remote.
pub(crate) fn sync_remote(
    repository: &GitRepository,
) -> CommandResult<Option<aeria_git::RemoteInfo>> {
    let remotes = repository.remotes()?;
    let upstream_remote = repository.status()?.upstream.and_then(|upstream| {
        upstream
            .split_once('/')
            .map(|(remote, _)| remote.to_owned())
    });
    let remote = upstream_remote
        .and_then(|name| remotes.iter().find(|remote| remote.name == name))
        .or_else(|| remotes.iter().find(|remote| remote.name == "origin"))
        .or_else(|| (remotes.len() == 1).then(|| &remotes[0]));
    Ok(remote.cloned())
}

/// See [`commit_web_url`].
fn web_url(remote: &str, commit: &str) -> Option<String> {
    let rest = if let Some(rest) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
        .or_else(|| remote.strip_prefix("ssh://"))
        .or_else(|| remote.strip_prefix("git://"))
    {
        let (authority, path) = rest.split_once('/')?;
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        // An SSH port is not the web port.
        let host = if remote.starts_with("ssh://") {
            host.split_once(':').map_or(host, |(host, _)| host)
        } else {
            host
        };
        format!("{host}/{path}")
    } else {
        // scp-like: `git@host:owner/repo.git`.
        let (authority, path) = remote.split_once(':')?;
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if host.is_empty() || host.contains(['/', '\\']) || host.len() == 1 || path.starts_with('/')
        {
            return None;
        }
        format!("{host}/{path}")
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (host, path) = rest.split_once('/')?;
    if host.is_empty() || path.is_empty() || !host.contains('.') {
        return None;
    }
    let commit_path = if host.contains("gitlab") {
        "-/commit"
    } else if host.contains("bitbucket") {
        "commits"
    } else {
        "commit"
    };
    Some(format!("https://{host}/{path}/{commit_path}/{commit}"))
}

#[tauri::command(rename_all = "camelCase")]
/// Opens a commit's page on the hosting service. The address comes from
/// the sync remote, never the renderer.
///
/// # Errors
///
/// Returns `gitNoWebPage` when the remote is not a hosting service, or
/// `openFailed`.
pub async fn git_open_commit(app: tauri::AppHandle, commit_id: String) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let state_app = app.clone();
    let url = run_blocking(move || {
        let state = state_app.state::<DesktopState>();
        let repository = open_repository(&state)?;
        let commit = repository.commit(&commit_id)?;
        commit_web_url(&repository, &commit.id)
    })
    .await?
    .ok_or_else(|| CommandError::new("gitNoWebPage", "the remote is not a hosting service"))?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| CommandError::new("openFailed", error.to_string()))
}

#[cfg(test)]
mod web_url_tests {
    use super::web_url;

    #[test]
    fn hosting_remotes_have_commit_pages() {
        let id = "abc123";
        assert_eq!(
            web_url("https://github.com/AngelicaProject/Prima.git", id).as_deref(),
            Some("https://github.com/AngelicaProject/Prima/commit/abc123")
        );
        assert_eq!(
            web_url("git@github.com:AngelicaProject/Prima.git", id).as_deref(),
            Some("https://github.com/AngelicaProject/Prima/commit/abc123")
        );
        assert_eq!(
            web_url("https://user:token@gitlab.com/group/sub/project", id).as_deref(),
            Some("https://gitlab.com/group/sub/project/-/commit/abc123")
        );
        assert_eq!(
            web_url("ssh://git@bitbucket.org:7999/team/repo.git", id).as_deref(),
            Some("https://bitbucket.org/team/repo/commits/abc123")
        );
        assert_eq!(web_url(r"D:\Projects\prima.git", id), None);
        assert_eq!(web_url("/srv/git/prima.git", id), None);
        assert_eq!(web_url("C:/Projects/prima.git", id), None);
    }
}
