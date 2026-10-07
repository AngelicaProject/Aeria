//! Operations on the working tree, the index, and history that Git tools
//! offer beside committing: staging and unstaging files, discarding their
//! changes, committing what is staged, undoing or amending the last commit
//! while it is only local, reverting a commit, starting a branch at a
//! commit, and searching history. See `docs/architecture/git.md`.

use std::fs;

use crate::entries::{EntryChange, diff_file, is_po_path, summarize_changes};
use crate::repository::{
    CheckpointOutcome, CommitSummary, FileChangeKind, GitRepository, LOG_FORMAT, parse_commit,
    validate_revision,
};
use crate::{GitError, merge_file};

/// A file's committed and staged content; `None` where there is none.
pub type FileVersions = (Option<Vec<u8>>, Option<Vec<u8>>);

/// Commits read when searching history.
const SEARCH_SCAN: usize = 50_000;

/// Checks project-relative paths before they reach Git: relative, without
/// `.` or `..` parts, and not empty. Git reads them literally
/// (`--literal-pathspecs`), so wildcards and pathspec magic have no meaning.
fn validate_paths(paths: &[String]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Err(GitError::InvalidInput {
            field: "paths",
            reason: "no file was given".to_owned(),
        });
    }
    for path in paths {
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.contains('\0')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(GitError::InvalidInput {
                field: "path",
                reason: format!("{path:?} is not a relative project path"),
            });
        }
    }
    Ok(())
}

impl GitRepository {
    /// Stages the current content of files, deletions included (`git add
    /// --all`).
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid path, or an error
    /// when Git fails.
    pub fn stage(&self, paths: &[String]) -> Result<(), GitError> {
        validate_paths(paths)?;
        let mut args = vec!["--literal-pathspecs", "add", "--all", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run(&args)?;
        Ok(())
    }

    /// Takes files out of the index, leaving the working tree as it is
    /// (`git restore --staged`; before the first commit, `git rm --cached`).
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid path, or an error
    /// when Git fails.
    pub fn unstage(&self, paths: &[String]) -> Result<(), GitError> {
        validate_paths(paths)?;
        let mut args = if self.head()?.is_some() {
            vec!["--literal-pathspecs", "restore", "--staged", "--"]
        } else {
            vec![
                "--literal-pathspecs",
                "rm",
                "--cached",
                "--quiet",
                "-r",
                "--",
            ]
        };
        args.extend(paths.iter().map(String::as_str));
        self.run(&args)?;
        Ok(())
    }

    /// Discards the working-tree changes of files: a tracked file gets back
    /// its staged or committed content (`git restore --worktree`), and an
    /// untracked file is deleted. Staged changes stay.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid path or a file with
    /// a merge conflict, or an error when Git or the file system fails.
    pub fn discard(&self, paths: &[String]) -> Result<(), GitError> {
        validate_paths(paths)?;
        let status = self.status()?;
        let mut restore = Vec::new();
        for path in paths {
            let Some(file) = status.files.iter().find(|file| &file.path == path) else {
                continue;
            };
            match file.worktree {
                Some(FileChangeKind::Conflicted) => {
                    return Err(GitError::InvalidInput {
                        field: "path",
                        reason: format!("{path} has a merge conflict"),
                    });
                }
                Some(FileChangeKind::Untracked) => {
                    let full = self.root().join(path);
                    fs::remove_file(&full).map_err(|source| GitError::Io {
                        operation: "delete untracked file",
                        path: full,
                        source,
                    })?;
                }
                Some(_) => restore.push(path.as_str()),
                None => {}
            }
        }
        if !restore.is_empty() {
            let mut args = vec!["--literal-pathspecs", "restore", "--worktree", "--"];
            args.extend(restore);
            self.run(&args)?;
        }
        Ok(())
    }

    /// Whether the index holds changes against `HEAD`.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn has_staged_changes(&self) -> Result<bool, GitError> {
        Ok(self.status()?.files.iter().any(|file| {
            file.index
                .is_some_and(|kind| kind != FileChangeKind::Conflicted)
        }))
    }

    /// The string changes the index holds against `HEAD`, per staged PO
    /// file in path order.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn staged_changes(&self) -> Result<Vec<EntryChange>, GitError> {
        let status = self.status()?;
        let paths: Vec<&str> = status
            .files
            .iter()
            .filter(|file| file.index.is_some() && is_po_path(&file.path))
            .map(|file| file.path.as_str())
            .collect();
        let has_head = status.head.is_some();
        let mut specs = Vec::with_capacity(paths.len() * 2);
        for path in &paths {
            let top = self.top_level_path(path);
            if has_head {
                specs.push(format!("HEAD:{top}"));
            }
            specs.push(format!(":0:{top}"));
        }
        let mut blobs = self.read_blobs(&specs)?.into_iter();
        let mut changes = Vec::new();
        for path in &paths {
            let before = if has_head {
                blobs.next().flatten()
            } else {
                None
            };
            let after = blobs.next().flatten();
            changes.extend(diff_file(path, before.as_deref(), after.as_deref()));
        }
        Ok(changes)
    }

    /// The committed (`HEAD`) and staged content of files, read in one pass;
    /// `None` where the commit or the index has no such file.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid path, or an error
    /// when Git fails.
    pub fn committed_and_staged(&self, paths: &[String]) -> Result<Vec<FileVersions>, GitError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        validate_paths(paths)?;
        let has_head = self.head()?.is_some();
        let mut specs = Vec::with_capacity(paths.len() * 2);
        for path in paths {
            let top = self.top_level_path(path);
            if has_head {
                specs.push(format!("HEAD:{top}"));
            }
            specs.push(format!(":0:{top}"));
        }
        let mut blobs = self.read_blobs(&specs)?.into_iter();
        Ok(paths
            .iter()
            .map(|_| {
                let committed = if has_head {
                    blobs.next().flatten()
                } else {
                    None
                };
                (committed, blobs.next().flatten())
            })
            .collect())
    }

    /// The working-tree content of a project file; `None` when there is no
    /// such file.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid path, or an error
    /// when the file cannot be read.
    pub fn working_file(&self, path: &str) -> Result<Option<Vec<u8>>, GitError> {
        validate_paths(std::slice::from_ref(&path.to_owned()))?;
        self.read_working(path)
    }

    /// The staged content of a project file; `None` when the index has no
    /// such file.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn file_in_index(&self, path: &str) -> Result<Option<Vec<u8>>, GitError> {
        validate_paths(std::slice::from_ref(&path.to_owned()))?;
        let spec = format!(":0:{}", self.top_level_path(path));
        Ok(self.read_blobs(&[spec])?.into_iter().next().flatten())
    }

    /// Commits what the index holds, every staged file whatever it is, as
    /// the translator identity. A blank `message` is replaced with a
    /// summary of the staged string changes.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::IdentityMissing`], [`GitError::MergeInProgress`],
    /// [`GitError::NothingToCommit`], or an error when Git fails.
    pub fn commit_staged(&self, message: Option<&str>) -> Result<CheckpointOutcome, GitError> {
        let mut args = self.identity_options()?;
        if self.merge_in_progress()? {
            return Err(GitError::MergeInProgress);
        }
        if !self.has_staged_changes()? {
            return Err(GitError::NothingToCommit);
        }
        let changes = self.staged_changes()?;
        let message = commit_message(message, &changes);
        args.extend(["commit", "--quiet", "-m", &message]);
        self.run(&args)?;
        Ok(CheckpointOutcome {
            commit: self.commit("HEAD")?,
            changes,
        })
    }

    /// Whether a remote-tracking branch, as last fetched, contains the
    /// commit: it was pushed, and changing it would need a forced push.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn is_published(&self, revision: &str) -> Result<bool, GitError> {
        validate_revision(revision)?;
        let text = self.run_text(&["branch", "--remotes", "--contains", revision])?;
        Ok(!text.trim().is_empty())
    }

    /// The commits of `HEAD` no remote-tracking branch contains, as last
    /// fetched: what a push would publish. Empty without a remote, since
    /// there is nowhere to push; at most 10,000 are read.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn unpublished_commits(&self) -> Result<std::collections::HashSet<String>, GitError> {
        if self.head()?.is_none() || self.remotes()?.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let text = self.run_text(&[
            "rev-list",
            "--max-count=10000",
            "HEAD",
            "--not",
            "--remotes",
        ])?;
        Ok(text.lines().map(str::to_owned).collect())
    }

    /// The last commit, when it can be undone or amended: it exists, it is
    /// not a merge, no merge is in progress, and no remote has it.
    fn local_last_commit(&self) -> Result<CommitSummary, GitError> {
        if self.merge_in_progress()? {
            return Err(GitError::MergeInProgress);
        }
        if self.head()?.is_none() {
            return Err(GitError::UnbornHead);
        }
        let commit = self.commit("HEAD")?;
        if commit.parents.len() > 1 {
            return Err(GitError::InvalidInput {
                field: "commit",
                reason: "the last commit is a merge".to_owned(),
            });
        }
        if self.is_published(&commit.id)? {
            return Err(GitError::CommitPublished);
        }
        Ok(commit)
    }

    /// Undoes the last commit while no remote has it: its changes return to
    /// the index and the working tree (`git reset --soft HEAD~1`, or
    /// removing the branch's only commit). Returns the commit undone.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::CommitPublished`], [`GitError::UnbornHead`],
    /// [`GitError::MergeInProgress`], [`GitError::InvalidInput`] for a merge
    /// commit, or an error when Git fails.
    pub fn undo_last_commit(&self) -> Result<CommitSummary, GitError> {
        let commit = self.local_last_commit()?;
        if commit.parents.is_empty() {
            self.run(&["update-ref", "-d", "HEAD"])?;
        } else {
            self.run(&["reset", "--soft", "--quiet", "HEAD~1"])?;
        }
        Ok(commit)
    }

    /// Replaces the last commit, while no remote has it, with one that also
    /// holds the new changes: what is staged, or else every change of the
    /// Aeria-managed paths, as a commit would take them. A blank `message`
    /// keeps the commit's message; without changes only the message
    /// changes.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::CommitPublished`], [`GitError::NothingToCommit`]
    /// when neither changes nor a new message are given, or another typed
    /// Git error.
    pub fn amend(&self, message: Option<&str>) -> Result<CheckpointOutcome, GitError> {
        let mut args = self.identity_options()?;
        self.local_last_commit()?;
        let message = message.map(str::trim).filter(|text| !text.is_empty());
        args.extend(["commit", "--quiet", "--amend"]);
        match message {
            Some(message) => args.extend(["-m", message]),
            None => args.push("--no-edit"),
        }
        let changes;
        if self.has_staged_changes()? {
            changes = self.staged_changes()?;
        } else {
            let paths = self.managed_paths()?;
            if self.has_managed_changes(&paths)? {
                changes = self.pending_changes()?;
                let mut add = vec!["add", "--all", "--"];
                add.extend(&paths);
                self.run(&add)?;
                args.extend(["--only", "--"]);
                args.extend(&paths);
            } else if message.is_some() {
                changes = Vec::new();
                args.push("--allow-empty");
            } else {
                return Err(GitError::NothingToCommit);
            }
        }
        self.run(&args)?;
        Ok(CheckpointOutcome {
            commit: self.commit("HEAD")?,
            changes,
        })
    }

    /// Reverts a commit with a new commit on the current branch. PO files
    /// that conflict are joined per string as Pull joins them; a string the
    /// commit changed that was changed again since, or any other conflict,
    /// leaves the branch as it was. `accept` validates the resulting
    /// project; on failure the revert is undone. Merge commits are not
    /// reverted. Returns the new commit.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::UncommittedTranslations`],
    /// [`GitError::MergeConflict`], [`GitError::TranslationConflicts`],
    /// [`GitError::IncomingRejected`], [`GitError::NothingToCommit`] when
    /// the revert changes nothing, or another typed Git error. The branch is
    /// unchanged after an error.
    pub fn revert<F>(&self, revision: &str, accept: F) -> Result<CommitSummary, GitError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let identity = self.identity_options()?;
        self.require_clean_translations()?;
        let Some(before) = self.head()? else {
            return Err(GitError::UnbornHead);
        };
        let target = self.commit(revision)?;
        if target.parents.len() > 1 {
            return Err(GitError::InvalidInput {
                field: "commit",
                reason: "merge commits cannot be reverted".to_owned(),
            });
        }
        let outcome = self.apply_revert(&target, identity);
        if outcome.is_err() {
            self.abandon_revert(&before)?;
            outcome?;
        }
        if let Err(reason) = accept() {
            self.run(&["reset", "--merge", "--quiet", &before])?;
            return Err(GitError::IncomingRejected { reason });
        }
        self.commit("HEAD")
    }

    fn apply_revert(
        &self,
        target: &CommitSummary,
        identity: Vec<&'static str>,
    ) -> Result<(), GitError> {
        let output = self.output(&["revert", "--no-commit", target.id.as_str()])?;
        if !output.status.success() {
            let conflicted = self.conflicted_files()?;
            if conflicted.is_empty() {
                crate::process::require_success(&["revert"], &output)?;
            }
            let mut conflicts = Vec::new();
            let mut joined = Vec::new();
            let mut other = Vec::new();
            for path in &conflicted {
                let top = self.top_level_path(path);
                let stages = if is_po_path(path) {
                    self.read_blobs(&[
                        format!(":1:{top}"),
                        format!(":2:{top}"),
                        format!(":3:{top}"),
                    ])?
                } else {
                    Vec::new()
                };
                let merged = match stages.as_slice() {
                    [base, Some(ours), Some(theirs)] => merge_file(
                        path,
                        base.as_deref(),
                        ours,
                        theirs,
                        &std::collections::BTreeMap::new(),
                    ),
                    _ => None,
                };
                match merged {
                    Some((text, found)) => {
                        conflicts.extend(found);
                        joined.push((path, text));
                    }
                    None => other.push(path.clone()),
                }
            }
            if !other.is_empty() {
                return Err(GitError::MergeConflict { files: other });
            }
            if !conflicts.is_empty() {
                return Err(GitError::TranslationConflicts { conflicts });
            }
            for (path, text) in joined {
                let full = self.root().join(path);
                fs::write(&full, text).map_err(|source| GitError::Io {
                    operation: "write reverted PO file",
                    path: full.clone(),
                    source,
                })?;
                self.run(&["add", "--", path])?;
            }
        }
        if self.run(&["diff", "--cached", "--quiet"]).is_ok() {
            return Err(GitError::NothingToCommit);
        }
        let message = format!(
            "Revert \"{}\"\n\nThis reverts commit {}.",
            target.subject, target.id
        );
        let mut args = identity;
        args.extend(["commit", "--quiet", "-m", &message]);
        self.run(&args)?;
        Ok(())
    }

    /// Leaves a failed revert: the index and working tree of `before`, with
    /// no revert in progress.
    fn abandon_revert(&self, before: &str) -> Result<(), GitError> {
        self.run(&["reset", "--merge", "--quiet", before])?;
        // A revert left without a commit keeps its state until it is quit.
        let _ = self.output(&["revert", "--quit"]);
        Ok(())
    }

    /// Creates a branch at a commit and switches to it. Translation changes
    /// must be committed first; `accept` validates the resulting project, and
    /// on failure the previous branch is restored and the new one deleted.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::UncommittedTranslations`],
    /// [`GitError::IncomingRejected`], [`GitError::InvalidInput`] for an
    /// invalid name or revision, or another typed Git error.
    pub fn create_branch_at<F>(&self, name: &str, revision: &str, accept: F) -> Result<(), GitError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        self.validate_branch_name(name)?;
        validate_revision(revision)?;
        self.require_clean_translations()?;
        let previous = self.current_branch()?;
        let previous_commit = self.head()?;
        self.run(&["switch", "--quiet", "-c", name, revision])?;
        if let Err(reason) = accept() {
            match (previous, previous_commit) {
                (Some(branch), _) => self.run(&["switch", "--quiet", &branch])?,
                (None, Some(commit)) => self.run(&["switch", "--quiet", "--detach", &commit])?,
                (None, None) => Vec::new(),
            };
            self.run(&["branch", "--quiet", "-D", name])?;
            return Err(GitError::IncomingRejected { reason });
        }
        Ok(())
    }

    /// History newest first, as [`Self::log`], narrowed to the commits that
    /// changed `path` and whose subject, author, or ID contains `query`,
    /// ignoring case. The newest 50,000 commits are searched.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid path, or an error
    /// when Git fails.
    pub fn search_log(
        &self,
        skip: usize,
        limit: usize,
        query: &str,
        path: Option<&str>,
    ) -> Result<Vec<CommitSummary>, GitError> {
        if self.head()?.is_none() {
            return Ok(Vec::new());
        }
        if let Some(path) = path {
            validate_paths(std::slice::from_ref(&path.to_owned()))?;
        }
        let scan = format!("--max-count={SEARCH_SCAN}");
        let mut args = vec![
            "--literal-pathspecs",
            "log",
            "-z",
            "--topo-order",
            LOG_FORMAT,
            &scan,
            "HEAD",
            "--",
        ];
        args.push(path.unwrap_or("."));
        let needle = query.trim().to_lowercase();
        let text = self.run_text(&args)?;
        let mut found = Vec::new();
        for record in text.split('\0').filter(|record| !record.is_empty()) {
            let commit = parse_commit(record)?;
            let matches = needle.is_empty()
                || commit.subject.to_lowercase().contains(&needle)
                || commit.author_name.to_lowercase().contains(&needle)
                || commit.author_email.to_lowercase().contains(&needle)
                || commit.id.starts_with(&needle);
            if matches {
                found.push(commit);
            }
        }
        Ok(found.into_iter().skip(skip).take(limit).collect())
    }
}

/// A commit message: the one given, or a summary of the string changes.
fn commit_message(message: Option<&str>, changes: &[EntryChange]) -> String {
    match message.map(str::trim).filter(|text| !text.is_empty()) {
        Some(message) => message.to_owned(),
        None if changes.is_empty() => "Update project files".to_owned(),
        None => summarize_changes(changes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_must_be_relative_project_paths() {
        assert!(validate_paths(&["po/Addon.po".to_owned()]).is_ok());
        for path in [
            "",
            "/etc/passwd",
            "../x",
            "po/../x",
            "po//x",
            r"po\x",
            "./po",
        ] {
            assert!(validate_paths(&[path.to_owned()]).is_err(), "{path}");
        }
        assert!(validate_paths(&[]).is_err());
    }
}
