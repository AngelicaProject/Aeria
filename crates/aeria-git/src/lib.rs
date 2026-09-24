//! Git repository operations and semantic collaboration helpers.
//!
//! Aeria uses Git as its collaboration and history layer. This crate drives
//! the system Git client for repository operations and interprets workspace
//! unit shards so history and changes can be presented per translation
//! unit instead of per file. The Git author identity is the translator
//! identity: every checkpoint is attributed to the configured
//! `user.name`/`user.email`.

#![forbid(unsafe_code)]

mod branches;
mod collaboration;
mod merge;
mod process;
mod repository;
mod semantic;
mod sync;

use std::path::PathBuf;

use aeria_workspace::WorkspaceStoreError;
use thiserror::Error;

pub use branches::{BranchInfo, ContributionStatus, FinishOutcome};
pub use collaboration::{COLLABORATION_FILE, CollaborationPolicy, CollaborationSettings};
pub use merge::{ConflictResolution, UnitConflict};
pub use process::{GitExecutable, GitOrigin};
pub use repository::{
    CheckpointOutcome, CommitSummary, ConfigScope, FileChangeKind, FileStatus, GitRepository,
    RemoteInfo, RepositoryStatus, TranslatorIdentity,
};
pub use semantic::{
    Attribution, ContributorSummary, RecordVersion, UnitAttribution, UnitChange, UnitChangeKind,
    UnitHistory, UnitRevision, summarize_changes, summarize_contributors,
};
pub use sync::{IntegrateOutcome, RECONCILE_MESSAGE};

/// Errors raised by Git collaboration operations.
#[derive(Debug, Error)]
pub enum GitError {
    /// The Git executable could not be started.
    #[error("could not run Git ({program}): {message}; install Git and make sure it is on PATH")]
    GitUnavailable { program: String, message: String },

    /// A Git command exited unsuccessfully.
    #[error("git {command} failed{}: {stderr}", status.map_or_else(String::new, |code| format!(" with exit code {code}")))]
    CommandFailed {
        command: String,
        status: Option<i32>,
        stderr: String,
    },

    /// Git output did not match the documented porcelain format.
    #[error("unexpected Git output: {message}")]
    Parse { message: String },

    /// The project root is not inside a Git working tree.
    #[error("{path} is not inside a Git repository")]
    NotARepository { path: PathBuf },

    /// The project root is already inside a Git working tree.
    #[error("{path} is already inside a Git repository")]
    AlreadyARepository { path: PathBuf },

    /// A value supplied by the user cannot be passed to Git safely.
    #[error("invalid {field}: {reason}")]
    InvalidInput { field: &'static str, reason: String },

    /// Commits require a translator name.
    #[error("translator name is not configured; set your name first")]
    IdentityMissing,

    /// There are no translation changes to checkpoint.
    #[error("there are no translation changes to checkpoint")]
    NothingToCommit,

    /// Sync requires translation changes to be checkpointed first.
    #[error("translation changes are not checkpointed; create a checkpoint before syncing")]
    UncommittedTranslations,

    /// The repository is not on a branch.
    #[error("the repository is not on a branch (detached HEAD)")]
    DetachedHead,

    /// The repository has no commits yet.
    #[error("the repository has no commits yet")]
    UnbornHead,

    /// A merge started outside Aeria is still in progress.
    #[error("a merge is already in progress; finish or abort it with Git first")]
    MergeInProgress,

    /// No remote is configured for sync.
    #[error("no remote is configured; add a remote before syncing")]
    NoRemote,

    /// Incoming changes conflict textually with local commits. The merge was
    /// aborted and the repository is unchanged.
    #[error("incoming changes conflict with local commits in {}", files.join(", "))]
    MergeConflict { files: Vec<String> },

    /// The same translation units were changed differently locally and
    /// remotely. The merge was aborted; retry with explicit resolutions.
    #[error("{} translated strings were changed differently here and on the remote", conflicts.len())]
    TranslationConflicts { conflicts: Vec<UnitConflict> },

    /// Collaboration settings are invalid or not applicable.
    #[error("invalid collaboration settings: {reason}")]
    InvalidSettings { reason: String },

    /// Incoming changes merged cleanly but were rejected by workspace
    /// validation. The merge was rolled back.
    #[error(
        "incoming changes were rolled back because the resulting project is not valid: {reason}"
    )]
    IncomingRejected { reason: String },

    /// A workspace file or historical revision is invalid.
    #[error(transparent)]
    Workspace(#[from] WorkspaceStoreError),

    /// A filesystem operation failed.
    #[error("filesystem operation '{operation}' failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
}
