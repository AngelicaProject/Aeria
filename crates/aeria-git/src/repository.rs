//! Repository-level Git operations: discovery, status, identity, remotes,
//! checkpoints, history, and sync.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::GitError;
use crate::collaboration::{COLLABORATION_FILE, CollaborationSettings};
use crate::process::{GitExecutable, require_success};
use crate::semantic::{UnitChange, summarize_changes};

pub(crate) const AERIA_PATH: &str = ".aeria";
pub const ATTRIBUTES_FILE: &str = ".gitattributes";
/// The project-root file of Pack Settings v1, owned by `aeria-export`.
pub const PACK_SETTINGS_FILE: &str = "aeria-pack.json";
/// The project-root file of Font Settings v1, owned by `aeria-fonts`.
pub const FONT_SETTINGS_FILE: &str = "aeria-fonts.json";
/// The project directory of source fonts named by the font settings.
pub const FONTS_DIR: &str = "fonts";
/// The project glossary, owned by `aeria-ai`.
pub const GLOSSARY_FILE: &str = "aeria-glossary.csv";
/// The project translation guidance, owned by `aeria-ai`.
pub const GUIDANCE_FILE: &str = "aeria-guidance.md";
/// The feed workflow, owned by `aeria-publish`. It builds the update feed on
/// GitHub from released packs, so it belongs to the project like its settings.
pub const FEED_WORKFLOW_FILE: &str = ".github/workflows/harmonia-feed.yml";
/// Every project path a checkpoint commits besides `.aeria/`.
pub const PROJECT_PATHS: [&str; 9] = [
    ATTRIBUTES_FILE,
    COLLABORATION_FILE,
    PACK_SETTINGS_FILE,
    FONT_SETTINGS_FILE,
    FONTS_DIR,
    GLOSSARY_FILE,
    GUIDANCE_FILE,
    FEED_WORKFLOW_FILE,
    crate::workflow::CHECK_WORKFLOW_FILE,
];
/// Workspace Format files are LF-only. This rule keeps Git from
/// converting them on checkout (for example with `core.autocrlf=true`).
const ATTRIBUTES_RULE: &str = "/.aeria/** text eol=lf";
const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s";

/// A Git working tree that contains an Aeria project root.
///
/// The project root may be the repository top level or a subdirectory of it.
/// Every command runs with the project root as its working directory.
#[derive(Clone, Debug)]
pub struct GitRepository {
    root: PathBuf,
    prefix: String,
    git: GitExecutable,
}

/// Where a Git configuration value comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigScope {
    /// System-wide configuration.
    System,
    /// The user's global configuration.
    Global,
    /// This repository's configuration.
    Repository,
    /// Per-worktree configuration.
    Worktree,
    /// Command-line or environment configuration.
    Command,
}

/// The translator identity: the Git author used for checkpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslatorIdentity {
    pub name: Option<String>,
    pub email: Option<String>,
    pub name_scope: Option<ConfigScope>,
    pub email_scope: Option<ConfigScope>,
}

impl TranslatorIdentity {
    /// Returns whether checkpoints can be attributed. Only the name is
    /// required; the email is optional.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.name.is_some()
    }
}

/// The working-tree change kind of one file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

/// One changed file, relative to the project root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStatus {
    pub path: String,
    pub original_path: Option<String>,
    pub kind: FileChangeKind,
    pub staged: bool,
}

impl FileStatus {
    /// Returns whether the file belongs to Aeria-managed workspace data.
    #[must_use]
    pub fn is_translation_data(&self) -> bool {
        is_aeria_path(&self.path)
    }
}

/// Repository status restricted to the project root.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepositoryStatus {
    /// Current branch, or `None` for a detached HEAD.
    pub branch: Option<String>,
    /// Current commit, or `None` before the first commit.
    pub head: Option<String>,
    /// Upstream branch such as `origin/main`.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub merge_in_progress: bool,
    pub files: Vec<FileStatus>,
}

impl RepositoryStatus {
    /// Returns whether Aeria-managed workspace data has uncommitted changes.
    #[must_use]
    pub fn has_translation_changes(&self) -> bool {
        self.files.iter().any(FileStatus::is_translation_data)
    }
}

/// A configured remote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
}

/// One commit in project history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSummary {
    pub id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Author time in seconds since the Unix epoch.
    pub authored_at: i64,
    /// Branch and tag names pointing at the commit, as `git log --decorate`
    /// prints them: `HEAD -> main`, `origin/main`, `tag: harmonia/3`.
    pub refs: Vec<String>,
    pub subject: String,
}

/// The result of a checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointOutcome {
    pub commit: CommitSummary,
    pub changes: Vec<UnitChange>,
    /// The contribution branch created for this checkpoint under the
    /// pull-request policy, if any.
    pub branch_created: Option<String>,
}

impl GitRepository {
    /// Finds the Git working tree containing `root`, or `None` when `root`
    /// is not inside one.
    ///
    /// # Errors
    ///
    /// Returns an error when `root` is not a directory or Git fails for a
    /// reason other than a missing repository.
    pub fn discover(
        root: impl Into<PathBuf>,
        git: GitExecutable,
    ) -> Result<Option<Self>, GitError> {
        let root = root.into();
        require_directory(&root)?;
        let args = ["rev-parse", "--is-inside-work-tree", "--show-prefix"];
        let output = git.output(&root, &args)?;
        if !output.status.success() {
            if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
                return Ok(None);
            }
            require_success(&args, &output)?;
        }
        let stdout = String::from_utf8(output.stdout).map_err(|_| GitError::Parse {
            message: "rev-parse produced non-UTF-8 output".to_owned(),
        })?;
        let mut lines = stdout.lines();
        if lines.next() != Some("true") {
            return Ok(None);
        }
        let prefix = lines.next().unwrap_or_default().to_owned();
        Ok(Some(Self { root, prefix, git }))
    }

    /// Opens the Git working tree containing `root`.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::NotARepository`] when `root` is not inside a Git
    /// working tree.
    pub fn open(root: impl Into<PathBuf>, git: GitExecutable) -> Result<Self, GitError> {
        let root = root.into();
        Self::discover(root.clone(), git)?.ok_or(GitError::NotARepository { path: root })
    }

    /// Initializes a new repository at `root` and adds the Aeria line-ending
    /// rule to `.gitattributes`. Nothing is committed.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::AlreadyARepository`] when `root` is already inside a
    /// Git working tree, or an error when Git or the filesystem fails.
    pub fn init(root: impl Into<PathBuf>, git: GitExecutable) -> Result<Self, GitError> {
        let root = root.into();
        if Self::discover(root.clone(), git.clone())?.is_some() {
            return Err(GitError::AlreadyARepository { path: root });
        }
        let configured_branch = git.output(&root, &["config", "--get", "init.defaultBranch"])?;
        let mut args = vec!["init", "--quiet"];
        if !configured_branch.status.success() {
            args.push("--initial-branch=main");
        }
        git.run(&root, &args)?;
        ensure_line_ending_rule(&root)?;
        Self::open(root, git)
    }

    /// Clones `url` into the new directory `destination`.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe URL, an existing non-empty destination,
    /// or a failed clone.
    pub fn clone_from(
        url: &str,
        destination: impl Into<PathBuf>,
        git: GitExecutable,
    ) -> Result<Self, GitError> {
        validate_url(url)?;
        let destination = destination.into();
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| GitError::InvalidInput {
                field: "clone destination",
                reason: "the destination must have a parent directory".to_owned(),
            })?;
        require_directory(parent)?;
        let args: Vec<OsString> = vec![
            "clone".into(),
            "--quiet".into(),
            "--".into(),
            url.into(),
            destination.clone().into_os_string(),
        ];
        git.run(parent, &args)?;
        Self::open(destination, git)
    }

    /// Clones `url` into a new folder inside `parent`, named after the
    /// repository by [`clone_folder_name`]. `parent` is created when missing.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe URL, a URL that names no folder, an
    /// existing non-empty destination, or a failed clone.
    pub fn clone_into(
        url: &str,
        parent: impl AsRef<Path>,
        git: GitExecutable,
    ) -> Result<Self, GitError> {
        validate_url(url)?;
        let name = clone_folder_name(url)?;
        let parent = parent.as_ref();
        fs::create_dir_all(parent).map_err(|source| GitError::Io {
            operation: "create clone parent directory",
            path: parent.to_owned(),
            source,
        })?;
        let destination = parent.join(name);
        let occupied = match fs::read_dir(&destination) {
            Ok(mut entries) => entries.next().is_some(),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        };
        if occupied {
            return Err(GitError::InvalidInput {
                field: "clone destination",
                reason: format!("{} already exists", destination.display()),
            });
        }
        Self::clone_from(url, destination, git)
    }

    /// Returns the project root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn git(&self) -> &GitExecutable {
        &self.git
    }

    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The project's folder relative to the repository's top folder: empty
    /// when they are the same, otherwise ending in `/`.
    #[must_use]
    pub fn project_prefix(&self) -> &str {
        &self.prefix
    }

    pub(crate) fn output<S: AsRef<std::ffi::OsStr>>(
        &self,
        args: &[S],
    ) -> Result<std::process::Output, GitError> {
        self.git.output(&self.root, args)
    }

    /// Converts a project-relative path to a repository top-level path.
    pub(crate) fn top_level_path(&self, project_path: &str) -> String {
        format!("{}{project_path}", self.prefix)
    }

    pub(crate) fn run<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Result<Vec<u8>, GitError> {
        self.git.run(&self.root, args)
    }

    pub(crate) fn run_text<S: AsRef<std::ffi::OsStr>>(
        &self,
        args: &[S],
    ) -> Result<String, GitError> {
        self.git.run_text(&self.root, args)
    }

    /// A short fingerprint of the repository state visible to the project:
    /// branch, `HEAD`, upstream, ahead/behind, and every changed or untracked
    /// file. It changes whenever anything a Git view shows may have changed,
    /// so a view can poll it cheaply and reload only then.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn state_stamp(&self) -> Result<String, GitError> {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let output = self.run(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "--",
            ".",
        ])?;
        let mut hasher = DefaultHasher::new();
        output.hash(&mut hasher);
        Ok(format!("{:016x}", hasher.finish()))
    }

    /// The content of a project-relative file in a remote-tracking branch
    /// (`<remote>/<branch>`) as of the last fetch; `None` when the branch or
    /// the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid path, or an error
    /// when Git fails.
    pub fn remote_file(
        &self,
        remote_branch: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, GitError> {
        if remote_branch.starts_with('-') || remote_branch.contains("..") {
            return Err(GitError::InvalidInput {
                field: "remote branch",
                reason: "is not a remote-tracking branch".to_owned(),
            });
        }
        match self.verify_ref(&format!("refs/remotes/{remote_branch}"))? {
            Some(commit) => self.file_at(&commit, path),
            None => Ok(None),
        }
    }

    /// The commit a local branch points at; `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid name, or an error
    /// when Git fails.
    pub fn branch_head(&self, branch: &str) -> Result<Option<String>, GitError> {
        self.validate_branch_name(branch)?;
        self.verify_ref(&format!("refs/heads/{branch}"))
    }

    /// Returns the names of local tags that start with `prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn tags_with_prefix(&self, prefix: &str) -> Result<Vec<String>, GitError> {
        let pattern = format!("{prefix}*");
        Ok(self
            .run_text(&["tag", "--list", "--", &pattern])?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Returns the current commit, or `None` before the first commit.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot be run.
    pub fn head(&self) -> Result<Option<String>, GitError> {
        self.verify_ref("HEAD")
    }

    pub(crate) fn verify_ref(&self, name: &str) -> Result<Option<String>, GitError> {
        let spec = format!("{name}^{{commit}}");
        let output = self
            .git
            .output(&self.root, &["rev-parse", "--quiet", "--verify", &spec])?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    }

    pub(crate) fn current_branch(&self) -> Result<Option<String>, GitError> {
        let output = self
            .git
            .output(&self.root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    }

    pub(crate) fn merge_in_progress(&self) -> Result<bool, GitError> {
        let output = self.git.output(
            &self.root,
            &["rev-parse", "--quiet", "--verify", "MERGE_HEAD"],
        )?;
        Ok(output.status.success())
    }

    /// Returns branch, upstream, and changed files under the project root.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or its porcelain output is malformed.
    pub fn status(&self) -> Result<RepositoryStatus, GitError> {
        let output = self.run(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ])?;
        let mut status = parse_status(&output, &self.prefix)?;
        status.merge_in_progress = self.merge_in_progress()?;
        Ok(status)
    }

    /// Returns the effective translator identity.
    ///
    /// # Errors
    ///
    /// Returns an error when Git configuration cannot be read.
    pub fn identity(&self) -> Result<TranslatorIdentity, GitError> {
        let name = self.config_get("user.name")?;
        let email = self.config_get("user.email")?;
        Ok(TranslatorIdentity {
            name_scope: name.as_ref().map(|(_, scope)| *scope),
            email_scope: email.as_ref().map(|(_, scope)| *scope),
            name: name.map(|(value, _)| value),
            email: email.map(|(value, _)| value),
        })
    }

    pub(crate) fn config_get(&self, key: &str) -> Result<Option<(String, ConfigScope)>, GitError> {
        let args = ["config", "--show-scope", "--get", key];
        let output = self.git.output(&self.root, &args)?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        require_success(&args, &output)?;
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.trim_end_matches(['\r', '\n']);
        let (scope, value) = line.split_once('\t').ok_or_else(|| GitError::Parse {
            message: format!("unexpected git config output {line:?}"),
        })?;
        let scope = match scope {
            "system" => ConfigScope::System,
            "global" => ConfigScope::Global,
            "local" => ConfigScope::Repository,
            "worktree" => ConfigScope::Worktree,
            _ => ConfigScope::Command,
        };
        let value = value.trim();
        Ok((!value.is_empty()).then(|| (value.to_owned(), scope)))
    }

    /// Sets the translator identity in this repository (`global == false`) or
    /// in the user's global Git configuration.
    ///
    /// The email is optional. Without one, an explicitly empty email is
    /// stored so Git never derives an address from the local user and host
    /// name.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an empty name or an unsafe
    /// value, or an error when Git configuration cannot be written.
    pub fn set_identity(
        &self,
        name: &str,
        email: Option<&str>,
        global: bool,
    ) -> Result<(), GitError> {
        let name = validate_identity_value("translator name", name)?;
        let email = match email.map(str::trim).filter(|email| !email.is_empty()) {
            Some(email) => validate_identity_value("translator email", email)?,
            None => "",
        };
        let scope = if global { "--global" } else { "--local" };
        self.run(&["config", scope, "--", "user.name", name])?;
        self.run(&["config", scope, "--", "user.email", email])?;
        Ok(())
    }

    /// Returns Git options that make commits use only the configured
    /// translator identity, with an empty email when none is configured.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::IdentityMissing`] when no name is configured.
    pub(crate) fn identity_options(&self) -> Result<Vec<&'static str>, GitError> {
        let identity = self.identity()?;
        if !identity.is_complete() {
            return Err(GitError::IdentityMissing);
        }
        let mut options = vec!["-c", "user.useConfigOnly=true"];
        if identity.email.is_none() {
            options.extend(["-c", "user.email="]);
        }
        Ok(options)
    }

    /// Returns the project-shared collaboration settings.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid or unreadable settings.
    pub fn collaboration(&self) -> Result<CollaborationSettings, GitError> {
        CollaborationSettings::load(&self.root)
    }

    /// Lists configured remotes with their fetch URLs.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn remotes(&self) -> Result<Vec<RemoteInfo>, GitError> {
        let text = self.run_text(&["remote", "-v"])?;
        let mut remotes: Vec<RemoteInfo> = Vec::new();
        for line in text.lines() {
            let Some((name, rest)) = line.split_once('\t') else {
                continue;
            };
            let Some(url) = rest.strip_suffix(" (fetch)") else {
                continue;
            };
            remotes.push(RemoteInfo {
                name: name.to_owned(),
                url: url.to_owned(),
            });
        }
        Ok(remotes)
    }

    /// Adds a remote or changes the URL of an existing one.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an unsafe name or URL, or an
    /// error when Git fails.
    pub fn set_remote(&self, name: &str, url: &str) -> Result<(), GitError> {
        self.validate_remote_name(name)?;
        validate_url(url)?;
        let exists = self.remotes()?.iter().any(|remote| remote.name == name);
        let action = if exists { "set-url" } else { "add" };
        self.run(&["remote", action, "--", name, url])?;
        Ok(())
    }

    /// Removes a remote and its remote-tracking branches.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid or unknown name, or
    /// an error when Git fails.
    pub fn remove_remote(&self, name: &str) -> Result<(), GitError> {
        self.validate_remote_name(name)?;
        if !self.remotes()?.iter().any(|remote| remote.name == name) {
            return Err(GitError::InvalidInput {
                field: "remote name",
                reason: "no such remote".to_owned(),
            });
        }
        self.run(&["remote", "remove", "--", name])?;
        Ok(())
    }

    /// Remote-tracking branches known from the last fetch, as
    /// `<remote>/<branch>`, sorted.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn remote_branches(&self) -> Result<Vec<String>, GitError> {
        let text = self.run_text(&[
            "for-each-ref",
            "--format=%(refname:strip=2)",
            "refs/remotes",
        ])?;
        let mut branches: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.ends_with("/HEAD") && name.contains('/'))
            .map(str::to_owned)
            .collect();
        branches.sort();
        Ok(branches)
    }

    /// Makes `remote_branch` (`<remote>/<branch>`, as listed by
    /// [`Self::remote_branches`]) the upstream of the current branch, which
    /// is where sync receives from and pushes to.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an unknown remote branch or a
    /// detached `HEAD`, or an error when Git fails.
    pub fn set_upstream(&self, remote_branch: &str) -> Result<(), GitError> {
        let invalid = |reason: &str| GitError::InvalidInput {
            field: "upstream",
            reason: reason.to_owned(),
        };
        if !self
            .remote_branches()?
            .iter()
            .any(|branch| branch == remote_branch)
        {
            return Err(invalid("is not a fetched remote branch"));
        }
        let Some(branch) = self.current_branch()? else {
            return Err(invalid("HEAD is not on a branch"));
        };
        let upstream = format!("--set-upstream-to=refs/remotes/{remote_branch}");
        self.run(&["branch", "--quiet", &upstream, "--", &branch])?;
        Ok(())
    }

    pub(crate) fn validate_remote_name(&self, name: &str) -> Result<(), GitError> {
        let invalid = |reason: &str| GitError::InvalidInput {
            field: "remote name",
            reason: reason.to_owned(),
        };
        if name.is_empty() || name.starts_with('-') {
            return Err(invalid("must be non-empty and must not start with '-'"));
        }
        let reference = format!("refs/remotes/{name}/probe");
        let output = self
            .git
            .output(&self.root, &["check-ref-format", &reference])?;
        if !output.status.success() {
            return Err(invalid("is not a valid Git remote name"));
        }
        Ok(())
    }

    /// Commits all Aeria-managed project data as the translator identity.
    ///
    /// Only `.aeria/`, `.gitattributes`, and the collaboration settings are
    /// committed; other staged or unstaged files are left untouched. A blank
    /// `message` is replaced with a deterministic summary of the
    /// translation-unit changes. Under the pull-request policy a checkpoint on
    /// the main branch first moves the uncommitted work to a new contribution
    /// branch.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::IdentityMissing`], [`GitError::NothingToCommit`],
    /// [`GitError::MergeInProgress`], or an error when Git or workspace
    /// decoding fails.
    pub fn checkpoint(&self, message: Option<&str>) -> Result<CheckpointOutcome, GitError> {
        let identity_options = self.identity_options()?;
        if self.merge_in_progress()? {
            return Err(GitError::MergeInProgress);
        }
        let changes = self.pending_changes()?;
        let paths = self.managed_paths()?;
        if !self.has_managed_changes(&paths)? {
            return Err(GitError::NothingToCommit);
        }

        // Work never lands on the main branch directly: a checkpoint there
        // moves the uncommitted work to a new contribution branch first. The
        // main branch comes from the committed settings, so committing a
        // settings change does not redirect its own checkpoint. Only the
        // first commit of a repository is made on the current branch.
        // The first commit of a repository creates the main branch. When the
        // project names a main branch other than the unborn one `git init`
        // chose (for example `main` against `init.defaultBranch=master`), the
        // unborn branch is renamed first, so the history starts on it.
        if self.head()?.is_none()
            && let Some(configured) = self.collaboration()?.main_branch
            && self.current_branch()?.as_deref() != Some(configured.as_str())
        {
            self.validate_branch_name(&configured)?;
            let reference = format!("refs/heads/{configured}");
            self.run(&["symbolic-ref", "HEAD", &reference])?;
        }
        let main = self.main_branch_from(&self.committed_collaboration()?)?;
        let branch_created = match (self.head()?, self.current_branch()?, main) {
            (Some(_), Some(current), Some(main)) if current == main => {
                let name = self.new_contribution_branch_name()?;
                self.run(&["switch", "--quiet", "-c", &name])?;
                Some(name)
            }
            _ => None,
        };

        let message = match message.map(str::trim).filter(|text| !text.is_empty()) {
            Some(message) => message.to_owned(),
            None if changes.is_empty() => "Update project settings".to_owned(),
            None => summarize_changes(&changes),
        };
        self.commit_managed_paths(identity_options, &paths, &message)?;

        let commit = self.commit("HEAD")?;
        Ok(CheckpointOutcome {
            commit,
            changes,
            branch_created,
        })
    }

    /// Stages and commits exactly the given Aeria-managed paths.
    fn commit_managed_paths(
        &self,
        identity_options: Vec<&'static str>,
        paths: &[&'static str],
        message: &str,
    ) -> Result<(), GitError> {
        let mut add = vec!["add", "--all", "--"];
        add.extend(paths);
        self.run(&add)?;
        let mut commit: Vec<&str> = identity_options;
        commit.extend(["commit", "--quiet", "-m", message, "--only", "--"]);
        commit.extend(paths);
        self.run(&commit)?;
        Ok(())
    }

    /// Returns the Aeria-managed paths that exist or are tracked.
    fn managed_paths(&self) -> Result<Vec<&'static str>, GitError> {
        let mut paths = vec![AERIA_PATH];
        for path in PROJECT_PATHS {
            if self.root.join(path).exists() || self.is_tracked(path)? {
                paths.push(path);
            }
        }
        Ok(paths)
    }

    fn has_managed_changes(&self, paths: &[&str]) -> Result<bool, GitError> {
        let mut args = vec!["status", "--porcelain", "--untracked-files=all", "--"];
        args.extend(paths);
        Ok(!self.run(&args)?.is_empty())
    }

    /// Writes project-shared collaboration settings (the main branch).
    /// Nothing is committed; the change is committed with the next
    /// checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid main branch, or an
    /// error when writing fails.
    pub fn set_collaboration(&self, settings: &CollaborationSettings) -> Result<(), GitError> {
        if let Some(branch) = &settings.main_branch {
            self.validate_branch_name(branch)?;
        }
        let text = settings.to_canonical_json();
        CollaborationSettings::parse(&text)?;
        let path = self.root.join(COLLABORATION_FILE);
        fs::write(&path, &text).map_err(|source| GitError::Io {
            operation: "write collaboration settings",
            path,
            source,
        })?;
        Ok(())
    }

    /// The main branch contributions are reviewed into: the configured one,
    /// else the default branch of the sync remote (`<remote>/HEAD`), else a
    /// local `main` or `master`, else the current branch of a repository
    /// without commits. `None` when none of these exists.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid settings or when Git fails.
    pub fn main_branch(&self) -> Result<Option<String>, GitError> {
        self.main_branch_from(&self.collaboration()?)
    }

    pub(crate) fn main_branch_from(
        &self,
        settings: &CollaborationSettings,
    ) -> Result<Option<String>, GitError> {
        if let Some(main) = &settings.main_branch {
            return Ok(Some(main.clone()));
        }
        let remote = match self.current_branch()? {
            Some(branch) => self.sync_remote(&branch).ok(),
            None => None,
        };
        if let Some(remote) = remote {
            let head = format!("refs/remotes/{remote}/HEAD");
            let output = self
                .git
                .output(&self.root, &["symbolic-ref", "--quiet", "--short", &head])?;
            if output.status.success() {
                let target = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if let Some(branch) = target.strip_prefix(&format!("{remote}/")) {
                    return Ok(Some(branch.to_owned()));
                }
            }
        }
        for candidate in ["main", "master"] {
            if self
                .verify_ref(&format!("refs/heads/{candidate}"))?
                .is_some()
                || self
                    .remote_branches()?
                    .iter()
                    .any(|name| name.ends_with(&format!("/{candidate}")))
            {
                return Ok(Some(candidate.to_owned()));
            }
        }
        if self.head()?.is_none() {
            return self.current_branch();
        }
        Ok(None)
    }

    /// The collaboration settings committed in `HEAD`; the default when the
    /// file is not committed.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or the committed file is invalid.
    pub fn committed_collaboration(&self) -> Result<CollaborationSettings, GitError> {
        match self.file_at("HEAD", COLLABORATION_FILE)? {
            None => Ok(CollaborationSettings::default()),
            Some(bytes) => CollaborationSettings::parse(&String::from_utf8_lossy(&bytes)),
        }
    }

    /// The content of a project-relative file in a commit; `None` when the
    /// commit does not have it or there are no commits yet.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid revision or path, or
    /// an error when Git fails.
    pub fn file_at(&self, revision: &str, path: &str) -> Result<Option<Vec<u8>>, GitError> {
        validate_revision(revision)?;
        if path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|part| part == ".." || part.is_empty())
        {
            return Err(GitError::InvalidInput {
                field: "path",
                reason: "must be a relative project path".to_owned(),
            });
        }
        if self.head()?.is_none() {
            return Ok(None);
        }
        let spec = format!("{revision}:{}", self.top_level_path(path));
        let output = self.git.output(&self.root, &["cat-file", "-e", &spec])?;
        if !output.status.success() {
            return Ok(None);
        }
        self.run(&["cat-file", "blob", &spec]).map(Some)
    }

    pub(crate) fn is_tracked(&self, path: &str) -> Result<bool, GitError> {
        let output = self
            .git
            .output(&self.root, &["ls-files", "--error-unmatch", "--", path])?;
        Ok(output.status.success())
    }

    /// Returns project history, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails or its output is malformed.
    pub fn log(&self, skip: usize, limit: usize) -> Result<Vec<CommitSummary>, GitError> {
        if self.head()?.is_none() {
            return Ok(Vec::new());
        }
        let skip = format!("--skip={skip}");
        let limit = format!("--max-count={limit}");
        // A project at the repository top level shows the complete history,
        // merges included; a subdirectory project shows its simplified
        // history with rewritten parents, so the graph stays connected.
        let mut args = vec![
            "log",
            "-z",
            "--topo-order",
            LOG_FORMAT,
            &skip,
            &limit,
            "HEAD",
        ];
        if !self.prefix.is_empty() {
            args.extend(["--parents", "--", "."]);
        }
        let text = self.run_text(&args)?;
        text.split('\0')
            .filter(|record| !record.is_empty())
            .map(parse_commit)
            .collect()
    }

    /// Returns one commit.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for a revision that is not `HEAD` or
    /// a hexadecimal object ID, or an error when the commit does not exist.
    pub fn commit(&self, revision: &str) -> Result<CommitSummary, GitError> {
        validate_revision(revision)?;
        let spec = format!("{revision}^{{commit}}");
        let text = self.run_text(&["log", "-1", "-z", LOG_FORMAT, &spec, "--"])?;
        parse_commit(text.trim_end_matches('\0'))
    }
}

pub(crate) fn is_aeria_path(path: &str) -> bool {
    path == AERIA_PATH || path.starts_with(".aeria/")
}

fn require_directory(path: &Path) -> Result<(), GitError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(GitError::Io {
            operation: "open directory",
            path: path.to_owned(),
            source: std::io::Error::other("not a directory"),
        }),
        Err(source) => Err(GitError::Io {
            operation: "open directory",
            path: path.to_owned(),
            source,
        }),
    }
}

fn ensure_line_ending_rule(root: &Path) -> Result<(), GitError> {
    let path = root.join(ATTRIBUTES_FILE);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(GitError::Io {
                operation: "read .gitattributes",
                path,
                source,
            });
        }
    };
    if existing.lines().any(|line| line.trim() == ATTRIBUTES_RULE) {
        return Ok(());
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str("# Aeria workspace data is LF-only.\n");
    updated.push_str(ATTRIBUTES_RULE);
    updated.push('\n');
    fs::write(&path, updated).map_err(|source| GitError::Io {
        operation: "write .gitattributes",
        path,
        source,
    })
}

fn validate_identity_value<'a>(field: &'static str, value: &'a str) -> Result<&'a str, GitError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(GitError::InvalidInput {
            field,
            reason: "must not be empty".to_owned(),
        });
    }
    if value
        .chars()
        .any(|character| character.is_control() || matches!(character, '<' | '>'))
    {
        return Err(GitError::InvalidInput {
            field,
            reason: "must not contain control characters, '<', or '>'".to_owned(),
        });
    }
    Ok(value)
}

fn validate_url(url: &str) -> Result<(), GitError> {
    let invalid = |reason: &str| GitError::InvalidInput {
        field: "remote URL",
        reason: reason.to_owned(),
    };
    if url.trim().is_empty() || url.starts_with('-') {
        return Err(invalid("must be non-empty and must not start with '-'"));
    }
    if url.trim() != url {
        return Err(invalid("must not start or end with whitespace"));
    }
    // Folder names may contain spaces, so a local path remote may too. Git
    // receives the URL as one argument, never through a shell.
    let local = is_local_path(url);
    if url.chars().any(|character| {
        character.is_control() || (character.is_whitespace() && !(local && character == ' '))
    }) {
        return Err(invalid(
            "must not contain control characters, or whitespace other than spaces in a local path",
        ));
    }
    if url.starts_with("ext::") || url.starts_with("fd::") {
        return Err(invalid("remote helper transports are not allowed"));
    }
    Ok(())
}

/// Whether `url` is a local filesystem path: absolute Unix (`/srv/x`),
/// Windows drive (`D:\x`, `D:/x`), or UNC (`\\server\share`).
fn is_local_path(url: &str) -> bool {
    let bytes = url.as_bytes();
    url.starts_with('/')
        || url.starts_with(r"\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
}

/// Returns the folder name `git clone` would choose for `url`: the last path
/// segment without a trailing `.git`.
///
/// # Errors
///
/// Returns an error when the URL names no usable folder, for example when the
/// segment is empty, ends with a dot or space (which includes `.`/`..`), or
/// contains characters Windows forbids.
pub fn clone_folder_name(url: &str) -> Result<String, GitError> {
    let path = url.trim().trim_end_matches(['/', '\\']);
    let path = path.strip_suffix("/.git").unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let name = path.rsplit(['/', '\\', ':']).next().unwrap_or_default();
    let forbidden = |character: char| character.is_control() || "<>:\"/\\|?*".contains(character);
    // Windows drops a trailing dot or space, so such a name is not the folder
    // the user expects.
    if name.is_empty() || name.ends_with(['.', ' ']) || name.contains(forbidden) {
        return Err(GitError::InvalidInput {
            field: "remote URL",
            reason: "does not end with a repository name to use as the folder name".to_owned(),
        });
    }
    Ok(name.to_owned())
}

pub(crate) fn validate_revision(revision: &str) -> Result<(), GitError> {
    let is_object_id = (4..=64).contains(&revision.len())
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if revision == "HEAD" || is_object_id {
        return Ok(());
    }
    Err(GitError::InvalidInput {
        field: "revision",
        reason: "expected HEAD or a lowercase hexadecimal commit ID".to_owned(),
    })
}

pub(crate) fn parse_commit(record: &str) -> Result<CommitSummary, GitError> {
    let fields: Vec<&str> = record.trim_start_matches('\n').split('\x1f').collect();
    let [
        id,
        parents,
        author_name,
        author_email,
        authored_at,
        refs,
        subject,
    ] = fields.as_slice()
    else {
        return Err(GitError::Parse {
            message: format!("unexpected commit record {record:?}"),
        });
    };
    let authored_at = authored_at.parse().map_err(|_| GitError::Parse {
        message: format!("invalid author time {authored_at:?}"),
    })?;
    Ok(CommitSummary {
        id: (*id).to_owned(),
        parents: parents.split_whitespace().map(str::to_owned).collect(),
        author_name: (*author_name).to_owned(),
        author_email: (*author_email).to_owned(),
        authored_at,
        refs: refs
            .split(", ")
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect(),
        subject: (*subject).to_owned(),
    })
}

pub(crate) fn strip_prefix(path: &str, prefix: &str) -> String {
    path.strip_prefix(prefix).unwrap_or(path).to_owned()
}

fn parse_status(output: &[u8], prefix: &str) -> Result<RepositoryStatus, GitError> {
    let text = std::str::from_utf8(output).map_err(|_| GitError::Parse {
        message: "git status produced non-UTF-8 output".to_owned(),
    })?;
    let mut status = RepositoryStatus::default();
    let mut entries = text.split('\0').filter(|entry| !entry.is_empty());
    let malformed = |entry: &str| GitError::Parse {
        message: format!("unexpected git status entry {entry:?}"),
    };
    while let Some(entry) = entries.next() {
        if let Some(header) = entry.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').ok_or_else(|| malformed(entry))?;
            match key {
                "branch.oid" if value != "(initial)" => status.head = Some(value.to_owned()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_owned()),
                "branch.upstream" => status.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    let mut counts = value.split(' ');
                    let ahead = counts.next().and_then(|count| count.strip_prefix('+'));
                    let behind = counts.next().and_then(|count| count.strip_prefix('-'));
                    status.ahead = ahead
                        .and_then(|count| count.parse().ok())
                        .ok_or_else(|| malformed(entry))?;
                    status.behind = behind
                        .and_then(|count| count.parse().ok())
                        .ok_or_else(|| malformed(entry))?;
                }
                _ => {}
            }
            continue;
        }
        let (kind, rest) = entry.split_at(1);
        let rest = rest.strip_prefix(' ').ok_or_else(|| malformed(entry))?;
        let file = match kind {
            "1" => {
                let fields: Vec<&str> = rest.splitn(8, ' ').collect();
                let [xy, _, _, _, _, _, _, path] = fields.as_slice() else {
                    return Err(malformed(entry));
                };
                tracked_file(xy, path, None, prefix).ok_or_else(|| malformed(entry))?
            }
            "2" => {
                let fields: Vec<&str> = rest.splitn(9, ' ').collect();
                let [xy, _, _, _, _, _, _, _, path] = fields.as_slice() else {
                    return Err(malformed(entry));
                };
                let original = entries.next().ok_or_else(|| malformed(entry))?;
                tracked_file(xy, path, Some(original), prefix).ok_or_else(|| malformed(entry))?
            }
            "u" => {
                let fields: Vec<&str> = rest.splitn(10, ' ').collect();
                let path = fields.last().filter(|_| fields.len() == 10);
                FileStatus {
                    path: strip_prefix(path.ok_or_else(|| malformed(entry))?, prefix),
                    original_path: None,
                    kind: FileChangeKind::Conflicted,
                    staged: false,
                }
            }
            "?" => FileStatus {
                path: strip_prefix(rest, prefix),
                original_path: None,
                kind: FileChangeKind::Untracked,
                staged: false,
            },
            "!" => continue,
            _ => return Err(malformed(entry)),
        };
        status.files.push(file);
    }
    Ok(status)
}

fn tracked_file(xy: &str, path: &str, original: Option<&str>, prefix: &str) -> Option<FileStatus> {
    let mut codes = xy.chars();
    let index = codes.next()?;
    let worktree = codes.next()?;
    let code = if worktree == '.' { index } else { worktree };
    let kind = match code {
        'A' => FileChangeKind::Added,
        'D' => FileChangeKind::Deleted,
        'R' => FileChangeKind::Renamed,
        'C' => FileChangeKind::Copied,
        'T' => FileChangeKind::TypeChanged,
        _ => FileChangeKind::Modified,
    };
    Some(FileStatus {
        path: strip_prefix(path, prefix),
        original_path: original.map(|original| strip_prefix(original, prefix)),
        kind,
        staged: index != '.',
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_path_remotes_may_contain_spaces_and_any_script() {
        for url in [
            r"D:\Общие проекты\перевод.git",
            "D:/Общие проекты/перевод.git",
            r"\\сервер\общая папка\перевод.git",
            "/home/пользователь/мои репозитории/перевод.git",
            "https://git.example/команда/перевод.git",
            "git@git.example:команда/перевод.git",
        ] {
            assert!(validate_url(url).is_ok(), "{url}");
        }
        for url in [
            "https://git.example/my repo.git",
            "git@git.example:team/my repo.git",
            "D:\\repo\tname.git",
            "D:\\repo\nname.git",
            " D:\\repo.git",
            "D:\\repo.git ",
            "ext::sh -c touch% /tmp/pwned",
        ] {
            assert!(validate_url(url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn clone_folder_names_follow_git_clone() {
        for (url, name) in [
            ("https://github.com/team/ffxiv-ru.git", "ffxiv-ru"),
            ("https://gitlab.com/team/ffxiv-ru/", "ffxiv-ru"),
            ("https://host/team/ffxiv-ru/.git", "ffxiv-ru"),
            ("git@github.com:team/ffxiv-ru.git", "ffxiv-ru"),
            ("ssh://git@host:2222/team/ffxiv-ru.git", "ffxiv-ru"),
            (r"D:\Remotes\ffxiv-ru.git", "ffxiv-ru"),
            ("file:///srv/git/ffxiv-ru", "ffxiv-ru"),
            ("https://git.example/команда/перевод.git", "перевод"),
            (r"D:\Общие проекты\перевод ffxiv.git", "перевод ffxiv"),
        ] {
            assert_eq!(clone_folder_name(url).expect(url), name, "{url}");
        }
        for url in [
            "",
            "https://",
            "https://host/..",
            "https://host/a?b",
            "https://host/name.",
        ] {
            assert!(clone_folder_name(url).is_err(), "{url}");
        }
    }

    #[test]
    fn porcelain_v2_status_is_parsed_relative_to_the_project() {
        let output = b"# branch.oid 0123456789abcdef0123456789abcdef01234567\0\
# branch.head main\0\
# branch.upstream origin/main\0\
# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaaa bbbb project/.aeria/units/7a.jsonl\0\
2 R. N... 100644 100644 100644 aaaa bbbb R100 project/new name.txt\0project/old.txt\0\
? project/.aeria/units/00.jsonl\0";
        let status = parse_status(output, "project/").expect("status");

        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        assert_eq!(status.files.len(), 3);
        assert_eq!(status.files[0].path, ".aeria/units/7a.jsonl");
        assert_eq!(status.files[0].kind, FileChangeKind::Modified);
        assert!(!status.files[0].staged);
        assert_eq!(status.files[1].path, "new name.txt");
        assert_eq!(status.files[1].original_path.as_deref(), Some("old.txt"));
        assert!(status.files[1].staged);
        assert_eq!(status.files[2].kind, FileChangeKind::Untracked);
        assert!(status.has_translation_changes());
    }

    #[test]
    fn unborn_detached_status_has_no_head_or_branch() {
        let status = parse_status(b"# branch.oid (initial)\0# branch.head (detached)\0", "")
            .expect("status");
        assert_eq!(status.head, None);
        assert_eq!(status.branch, None);
        assert!(!status.has_translation_changes());
    }

    #[test]
    fn unsafe_user_values_are_rejected_before_reaching_git() {
        assert!(validate_url("--upload-pack=evil").is_err());
        assert!(validate_url("ext::sh -c evil").is_err());
        assert!(validate_url("https://example.invalid/a b").is_err());
        assert!(validate_url("https://example.invalid/repo.git").is_ok());
        assert!(validate_identity_value("name", "  ").is_err());
        assert!(validate_identity_value("name", "A <b>").is_err());
        assert_eq!(
            validate_identity_value("name", " Ada ").expect("valid"),
            "Ada"
        );
        assert!(validate_revision("HEAD~1").is_err());
        assert!(validate_revision("--all").is_err());
        assert!(validate_revision("0123abcd").is_ok());
    }
}
