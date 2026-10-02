//! Branches: listing, creating, switching, and deleting them.

use crate::GitError;
use crate::repository::GitRepository;

/// One branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchInfo {
    /// Local name (`main`) or remote-tracking name (`origin/main`).
    pub name: String,
    pub remote: bool,
    pub current: bool,
    /// Upstream of a local branch, such as `origin/main`.
    pub upstream: Option<String>,
}

impl GitRepository {
    /// Lists local and remote-tracking branches.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails.
    pub fn branches(&self) -> Result<Vec<BranchInfo>, GitError> {
        let text = self.run_text(&[
            "for-each-ref",
            "--format=%(refname)%00%(upstream:short)%00%(HEAD)",
            "refs/heads",
            "refs/remotes",
        ])?;
        let mut branches = Vec::new();
        for line in text.lines() {
            let mut fields = line.split('\0');
            let (Some(reference), Some(upstream), Some(head)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let (name, remote) = if let Some(name) = reference.strip_prefix("refs/heads/") {
                (name, false)
            } else if let Some(name) = reference.strip_prefix("refs/remotes/") {
                if name.ends_with("/HEAD") {
                    continue;
                }
                (name, true)
            } else {
                continue;
            };
            branches.push(BranchInfo {
                name: name.to_owned(),
                remote,
                current: head == "*",
                upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
            });
        }
        Ok(branches)
    }

    pub(crate) fn validate_branch_name(&self, name: &str) -> Result<(), GitError> {
        let invalid = GitError::InvalidInput {
            field: "branch name",
            reason: "is not a valid Git branch name".to_owned(),
        };
        if name.is_empty() || name.starts_with('-') {
            return Err(invalid);
        }
        if !self
            .output(&["check-ref-format", "--branch", name])?
            .status
            .success()
        {
            return Err(invalid);
        }
        Ok(())
    }

    /// Creates a branch at the current commit and switches to it. Uncommitted
    /// work moves with it, so the workspace does not change.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid name or an error when
    /// the branch exists or Git fails.
    pub fn create_branch(&self, name: &str) -> Result<(), GitError> {
        self.validate_branch_name(name)?;
        self.run(&["switch", "--quiet", "-c", name])?;
        Ok(())
    }

    /// Switches to an existing local branch, or creates a tracking branch
    /// for a remote-only branch of the same name. Translation changes must be
    /// checkpointed first. `accept` validates the resulting project; on
    /// failure the previous branch is restored.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::UncommittedTranslations`],
    /// [`GitError::IncomingRejected`], or another typed Git error.
    pub fn switch_branch<F>(&self, name: &str, accept: F) -> Result<(), GitError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        self.validate_branch_name(name)?;
        self.require_clean_translations()?;
        let previous = self.current_branch()?;
        let previous_commit = self.head()?;
        self.run(&["switch", "--quiet", name])?;
        if let Err(reason) = accept() {
            match (previous, previous_commit) {
                (Some(branch), _) => self.run(&["switch", "--quiet", &branch])?,
                (None, Some(commit)) => self.run(&["switch", "--quiet", "--detach", &commit])?,
                (None, None) => Vec::new(),
            };
            return Err(GitError::IncomingRejected { reason });
        }
        Ok(())
    }

    /// Commits of a local branch that would be lost by deleting it, and its
    /// upstream on the remote too when `with_upstream`: commits no other
    /// branch, remote-tracking branch, or tag contains.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an invalid or unknown branch,
    /// or an error when Git fails.
    pub fn lost_commits(&self, branch: &str, with_upstream: bool) -> Result<u32, GitError> {
        self.validate_branch_name(branch)?;
        let reference = format!("refs/heads/{branch}");
        if self.verify_ref(&reference)?.is_none() {
            return Err(unknown_branch());
        }
        let mut leaving = vec![reference.clone()];
        if with_upstream && let Some(upstream) = self.upstream(branch)? {
            leaving.push(format!("refs/remotes/{upstream}"));
        }
        self.commits_only_in(&reference, &leaving)
    }

    /// Commits of a remote-tracking branch (`origin/feature`) that would be
    /// lost by deleting the branch on the remote: commits no other branch,
    /// remote-tracking branch, or tag contains.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an unknown branch, or an error
    /// when Git fails.
    pub fn lost_remote_commits(&self, remote_branch: &str) -> Result<u32, GitError> {
        self.split_remote_branch(remote_branch)?;
        let reference = format!("refs/remotes/{remote_branch}");
        self.commits_only_in(&reference, std::slice::from_ref(&reference))
    }

    /// Deletes a local branch other than the current one, and with
    /// `with_upstream` also its upstream branch on the remote. The remote
    /// branch goes first, so a remote that refuses (for example for its
    /// default branch) leaves the local branch in place. Without `force`, a
    /// deletion that would lose commits (see [`Self::lost_commits`]) is
    /// refused.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for the current or an unknown
    /// branch, or for one with commits that exist nowhere else without
    /// `force`; or an error when Git or the push fails.
    pub fn delete_branch(
        &self,
        branch: &str,
        with_upstream: bool,
        force: bool,
    ) -> Result<(), GitError> {
        if self.current_branch()?.as_deref() == Some(branch) {
            return Err(GitError::InvalidInput {
                field: "branch",
                reason: "is the current branch".to_owned(),
            });
        }
        if !force && self.lost_commits(branch, with_upstream)? > 0 {
            return Err(unmerged_branch());
        }
        if with_upstream && let Some(upstream) = self.upstream(branch)? {
            self.delete_on_remote(&upstream)?;
        }
        self.validate_branch_name(branch)?;
        self.run(&["branch", "--quiet", "-D", "--", branch])?;
        Ok(())
    }

    /// Deletes a branch on its remote, given by its remote-tracking name
    /// (`origin/feature`). Without `force`, a deletion that would lose
    /// commits (see [`Self::lost_remote_commits`]) is refused.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidInput`] for an unknown branch, or for one
    /// with commits that exist nowhere else without `force`; or an error
    /// when the push fails, for example because the remote protects the
    /// branch.
    pub fn delete_remote_branch(&self, remote_branch: &str, force: bool) -> Result<(), GitError> {
        if !force && self.lost_remote_commits(remote_branch)? > 0 {
            return Err(unmerged_branch());
        }
        self.delete_on_remote(remote_branch)
    }

    /// `origin/feature` as the remote `origin` and the branch `feature`, for
    /// a remote-tracking branch that exists.
    fn split_remote_branch(&self, remote_branch: &str) -> Result<(String, String), GitError> {
        let found = self.remotes()?.into_iter().find_map(|remote| {
            remote_branch
                .strip_prefix(&format!("{}/", remote.name))
                .map(|branch| (remote.name.clone(), branch.to_owned()))
        });
        let Some((remote, branch)) = found else {
            return Err(unknown_branch());
        };
        self.validate_branch_name(&branch)?;
        if self
            .verify_ref(&format!("refs/remotes/{remote_branch}"))?
            .is_none()
        {
            return Err(unknown_branch());
        }
        Ok((remote, branch))
    }

    /// Deletes the branch behind a remote-tracking branch on its remote. Git
    /// removes the remote-tracking branch with it; a recorded default branch
    /// of the remote (`<remote>/HEAD`) that named it is removed too.
    fn delete_on_remote(&self, remote_branch: &str) -> Result<(), GitError> {
        let (remote, branch) = self.split_remote_branch(remote_branch)?;
        let reference = format!("refs/heads/{branch}");
        self.run(&["push", "--quiet", &remote, "--delete", &reference])?;
        let head = format!("refs/remotes/{remote}/HEAD");
        let target = self.output(&["symbolic-ref", "--quiet", &head])?.stdout;
        if String::from_utf8_lossy(&target).trim() == format!("refs/remotes/{remote_branch}") {
            self.run(&["symbolic-ref", "--delete", &head])?;
        }
        Ok(())
    }

    /// Counts the commits of `reference` that no branch, remote-tracking
    /// branch, or tag outside `leaving` contains.
    fn commits_only_in(&self, reference: &str, leaving: &[String]) -> Result<u32, GitError> {
        let refs = self.run_text(&[
            "for-each-ref",
            "--format=%(refname)%00%(symref)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        let mut input = String::new();
        for line in refs.lines() {
            let mut fields = line.split('\0');
            let (Some(name), Some(symref)) = (fields.next(), fields.next()) else {
                continue;
            };
            if symref.is_empty() && !leaving.iter().any(|gone| gone == name) {
                input.push('^');
                input.push_str(name);
                input.push('\n');
            }
        }
        let args = ["rev-list", "--count", "--stdin", reference];
        let output = self
            .git()
            .output_with_input(self.root(), &args, input.into_bytes())?;
        crate::process::require_success(&args, &output)?;
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .map_err(|_| GitError::Parse {
                message: "unexpected rev-list count output".to_owned(),
            })
    }
}

fn unknown_branch() -> GitError {
    GitError::InvalidInput {
        field: "branch",
        reason: "does not exist".to_owned(),
    }
}

fn unmerged_branch() -> GitError {
    GitError::InvalidInput {
        field: "branch",
        reason: "has commits that exist on no other branch".to_owned(),
    }
}
