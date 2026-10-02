//! Git repository operations and semantic collaboration helpers.
//!
//! Aeria uses Git as its collaboration and history layer. This crate drives
//! the system Git client for repository operations and reads the project's
//! PO files so history and changes can be presented per string instead of
//! per file. The Git author identity is the translator
//! identity: every checkpoint is attributed to the configured
//! `user.name`/`user.email`.

#![forbid(unsafe_code)]

mod branches;
mod credential;
mod entries;
mod process;
mod repository;
mod sync;
mod workflow;

use std::path::PathBuf;

use thiserror::Error;

pub use branches::BranchInfo;
pub use credential::HostCredential;
pub use entries::{
    ConflictResolution, EntryChange, EntryChangeKind, EntryConflict, EntryHistory, EntryRevision,
    EntryState, PO_DIR, PendingCache, PendingFile, diff_file, is_po_path, merge_file,
    summarize_changes,
};
pub use process::{GitExecutable, GitOrigin};
pub use repository::{
    ATTRIBUTES_FILE, CheckpointOutcome, CommitSummary, ConfigScope, FEED_WORKFLOW_FILE,
    FONT_SETTINGS_FILE, FONTS_DIR, FileChangeKind, FileStatus, GitRepository, KNOWLEDGE_DIR,
    PACK_SETTINGS_FILE, PROJECT_PATHS, RemoteInfo, RepositoryStatus, SETTINGS_FILE,
    TranslatorIdentity, clone_folder_name,
};
pub use sync::IntegrateOutcome;
pub use workflow::{
    CHECK_WORKFLOW_FILE, CheckRelease, CheckWorkflowState, check_workflow_state,
    install_check_workflow, render_check_workflow,
};

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
    #[error("there are no translation changes to commit")]
    NothingToCommit,

    /// Integration requires translation changes to be checkpointed first.
    #[error("translation changes are not committed; commit them first")]
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

    /// No remote is configured to fetch from or push to.
    #[error("no remote is configured; add a remote first")]
    NoRemote,

    /// Incoming changes conflict textually with local commits. The merge was
    /// aborted and the repository is unchanged.
    #[error("incoming changes conflict with local commits in {}", files.join(", "))]
    MergeConflict { files: Vec<String> },

    /// The same strings were changed differently locally and remotely. The
    /// merge was aborted; retry with explicit resolutions.
    #[error("{} translated strings were changed differently here and on the remote", conflicts.len())]
    TranslationConflicts { conflicts: Vec<EntryConflict> },

    /// Incoming changes merged cleanly but were rejected by workspace
    /// validation. The merge was rolled back.
    #[error(
        "incoming changes were rolled back because the resulting project is not valid: {reason}"
    )]
    IncomingRejected { reason: String },

    /// A filesystem operation failed.
    #[error("filesystem operation '{operation}' failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
}
