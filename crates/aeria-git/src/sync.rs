//! Fetch, integrate, and push.
//!
//! Integration merges the upstream atomically: either the merge succeeds and
//! the caller accepts the result, or the branch is reset to where it
//! started. A PO file both sides changed is joined per
//! string; only strings both sides changed differently are reported.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::GitError;
use crate::entries::{ConflictResolution, is_po_path, merge_file};
use crate::repository::{GitRepository, strip_prefix};

/// How incoming commits were integrated.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IntegrateOutcome {
    /// Nothing to integrate.
    UpToDate,
    /// The branch fast-forwarded.
    FastForward,
    /// Local and incoming commits were merged with a merge commit.
    Merged,
}

impl IntegrateOutcome {
    /// Returns whether the working tree may have changed.
    #[must_use]
    pub const fn changed_working_tree(self) -> bool {
        !matches!(self, Self::UpToDate)
    }
}

impl GitRepository {
    pub(crate) fn require_branch(&self) -> Result<String, GitError> {
        self.current_branch()?.ok_or(GitError::DetachedHead)
    }

    /// Returns the remote used to sync `branch`: its configured remote,
    /// otherwise `origin`, otherwise the only remote.
    pub(crate) fn sync_remote(&self, branch: &str) -> Result<String, GitError> {
        let key = format!("branch.{branch}.remote");
        if let Some((remote, _)) = self.config_get(&key)? {
            return Ok(remote);
        }
        let remotes = self.remotes()?;
        if let Some(origin) = remotes.iter().find(|remote| remote.name == "origin") {
            return Ok(origin.name.clone());
        }
        match remotes.as_slice() {
            [only] => Ok(only.name.clone()),
            _ => Err(GitError::NoRemote),
        }
    }

    /// Fetches the sync remote of the current branch.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::DetachedHead`], [`GitError::NoRemote`], or an error
    /// when the fetch fails.
    pub fn fetch(&self) -> Result<(), GitError> {
        let branch = self.require_branch()?;
        let remote = self.sync_remote(&branch)?;
        self.run(&["fetch", "--prune", "--quiet", &remote])?;
        Ok(())
    }

    /// Fetches every remote, for listing their branches.
    ///
    /// # Errors
    ///
    /// Returns an error when Git fails, for example without network access.
    pub fn fetch_all(&self) -> Result<(), GitError> {
        if !self.remotes()?.is_empty() {
            self.run(&["fetch", "--all", "--prune", "--quiet"])?;
        }
        Ok(())
    }

    /// Integrates the already fetched commits of the current branch's
    /// upstream.
    ///
    /// Translation changes must be checkpointed first. PO files that
    /// conflict textually are joined per string (see
    /// [`crate::entries::merge_file`]). A string both sides changed
    /// differently is resolved only by its entry in `resolutions`, keyed by
    /// `msgctxt`; otherwise the merge is aborted and the strings are
    /// returned as [`GitError::TranslationConflicts`]. Once everything
    /// merged, `accept` must validate the resulting project; on failure the
    /// branch is reset to its starting commit.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::TranslationConflicts`], [`GitError::MergeConflict`],
    /// [`GitError::IncomingRejected`], [`GitError::UncommittedTranslations`],
    /// or another typed Git error. The branch is unchanged after an error.
    pub fn integrate<F>(
        &self,
        resolutions: &BTreeMap<String, ConflictResolution>,
        accept: F,
    ) -> Result<IntegrateOutcome, GitError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let branch = self.require_branch()?;
        self.require_clean_translations()?;

        let Some(upstream) = self.upstream(&branch)? else {
            return Ok(IntegrateOutcome::UpToDate);
        };
        let Some(before) = self.head()? else {
            return Err(GitError::UnbornHead);
        };

        let outcome = match self.merge_from(&upstream, resolutions) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.reset_to(&before)?;
                return Err(error);
            }
        };
        if outcome.changed_working_tree()
            && let Err(reason) = accept()
        {
            self.reset_to(&before)?;
            return Err(GitError::IncomingRejected { reason });
        }
        Ok(outcome)
    }

    pub(crate) fn require_clean_translations(&self) -> Result<(), GitError> {
        if self.merge_in_progress()? {
            return Err(GitError::MergeInProgress);
        }
        if self.status()?.has_translation_changes() {
            return Err(GitError::UncommittedTranslations);
        }
        Ok(())
    }

    pub(crate) fn reset_to(&self, commit: &str) -> Result<(), GitError> {
        if self.merge_in_progress()? {
            self.run(&["merge", "--abort"])?;
        }
        if self.head()?.as_deref() != Some(commit) {
            self.run(&["reset", "--merge", "--quiet", commit])?;
        }
        Ok(())
    }

    pub(crate) fn merge_from(
        &self,
        source: &str,
        resolutions: &BTreeMap<String, ConflictResolution>,
    ) -> Result<IntegrateOutcome, GitError> {
        let (ahead, behind) = self.ahead_behind(source)?;
        if behind == 0 {
            return Ok(IntegrateOutcome::UpToDate);
        }
        if ahead == 0 {
            self.run(&["merge", "--ff-only", "--quiet", source])?;
            return Ok(IntegrateOutcome::FastForward);
        }

        let mut args = self.identity_options()?;
        args.extend(["merge", "--no-edit", "--no-ff", "--quiet", source]);
        let output = self.output(&args)?;
        if output.status.success() {
            return Ok(IntegrateOutcome::Merged);
        }
        if !self.merge_in_progress()? {
            crate::process::require_success(&args, &output)?;
        }
        let conflicted = self.conflicted_files()?;
        if conflicted.is_empty() {
            crate::process::require_success(&args, &output)?;
        }
        let mut conflicts = Vec::new();
        let mut merged = Vec::new();
        let mut other = Vec::new();
        for path in &conflicted {
            if !is_po_path(path) {
                other.push(path.clone());
                continue;
            }
            let top = self.top_level_path(path);
            let stages = self.read_blobs(&[
                format!(":1:{top}"),
                format!(":2:{top}"),
                format!(":3:{top}"),
            ])?;
            let joined = match stages.as_slice() {
                [base, Some(ours), Some(theirs)] => {
                    merge_file(path, base.as_deref(), ours, theirs, resolutions)
                }
                _ => None,
            };
            match joined {
                Some((text, found)) => {
                    conflicts.extend(found);
                    merged.push((path, text));
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
        for (path, text) in merged {
            let full = self.root().join(path);
            fs::write(&full, text).map_err(|source| GitError::Io {
                operation: "write merged PO file",
                path: full.clone(),
                source,
            })?;
            self.run(&["add", "--", path])?;
        }
        let mut commit = self.identity_options()?;
        commit.extend(["commit", "--no-edit", "--quiet"]);
        self.run(&commit)?;
        Ok(IntegrateOutcome::Merged)
    }

    /// Pushes local commits of the current branch. Without an upstream, the
    /// branch is published to the sync remote and tracked. Returns whether
    /// anything was pushed.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::DetachedHead`], [`GitError::UnbornHead`],
    /// [`GitError::NoRemote`], or an error when the push fails (for example
    /// because the remote has new commits or protects the branch).
    pub fn push(&self) -> Result<bool, GitError> {
        let branch = self.require_branch()?;
        if self.head()?.is_none() {
            return Err(GitError::UnbornHead);
        }
        let remote = self.sync_remote(&branch)?;
        if let Some(upstream) = self.upstream(&branch)? {
            let (ahead, _) = self.ahead_behind(&upstream)?;
            if ahead == 0 {
                return Ok(false);
            }
            let merge_key = format!("branch.{branch}.merge");
            let (merge_ref, _) = self
                .config_get(&merge_key)?
                .ok_or_else(|| GitError::Parse {
                    message: format!("{merge_key} is not configured"),
                })?;
            let refspec = format!("HEAD:{merge_ref}");
            self.run(&["push", "--quiet", &remote, &refspec])
                .map_err(|error| protected(error, &remote, &branch))?;
        } else {
            let refspec = format!("HEAD:refs/heads/{branch}");
            self.run(&["push", "--quiet", "--set-upstream", &remote, &refspec])
                .map_err(|error| protected(error, &remote, &branch))?;
        }
        Ok(true)
    }

    /// Returns the upstream of `branch`, adopting `<remote>/<branch>` as the
    /// upstream when it exists and none is configured yet.
    pub(crate) fn upstream(&self, branch: &str) -> Result<Option<String>, GitError> {
        let spec = format!("{branch}@{{upstream}}");
        let output = self.output(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", &spec])?;
        if output.status.success() {
            return Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            ));
        }
        let remote = match self.sync_remote(branch) {
            Ok(remote) => remote,
            Err(GitError::NoRemote) => return Ok(None),
            Err(error) => return Err(error),
        };
        let candidate = format!("{remote}/{branch}");
        if self
            .verify_ref(&format!("refs/remotes/{candidate}"))?
            .is_none()
        {
            return Ok(None);
        }
        if self.head()?.is_some() {
            let upstream = format!("--set-upstream-to={candidate}");
            self.run(&["branch", "--quiet", &upstream])?;
        }
        Ok(Some(candidate))
    }

    pub(crate) fn ahead_behind(&self, other: &str) -> Result<(u32, u32), GitError> {
        let range = format!("HEAD...{other}");
        let text = self.run_text(&["rev-list", "--left-right", "--count", &range])?;
        let mut counts = text.split_whitespace().map(str::parse::<u32>);
        match (counts.next(), counts.next()) {
            (Some(Ok(ahead)), Some(Ok(behind))) => Ok((ahead, behind)),
            _ => Err(GitError::Parse {
                message: format!("unexpected rev-list count output {text:?}"),
            }),
        }
    }

    pub(crate) fn conflicted_files(&self) -> Result<Vec<String>, GitError> {
        let text = self.run_text(&["diff", "--name-only", "--diff-filter=U", "-z"])?;
        let files: BTreeSet<String> = text
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(|path| strip_prefix(path, self.prefix()))
            .collect();
        Ok(files.into_iter().collect())
    }
}

/// A push the remote refused by its branch rules, recognized by what GitHub
/// (`GH006`, `GH013`) and other hosts write for a protected branch.
fn protected(error: GitError, remote: &str, branch: &str) -> GitError {
    match &error {
        GitError::CommandFailed { stderr, .. } if is_protection_refusal(stderr) => {
            GitError::BranchProtected {
                remote: remote.to_owned(),
                branch: branch.to_owned(),
            }
        }
        _ => error,
    }
}

fn is_protection_refusal(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    text.contains("gh006")
        || text.contains("gh013")
        || text.contains("protected branch")
        || text.contains("repository rule violations")
        || text.contains("changes must be made through a pull request")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_by_branch_rules_are_recognized() {
        let refused = |stderr: &str| GitError::CommandFailed {
            command: "push".to_owned(),
            status: Some(1),
            stderr: stderr.to_owned(),
        };
        for stderr in [
            "remote: error: GH013: Repository rule violations found for refs/heads/main.\nremote: - Changes must be made through a pull request.",
            "remote: error: GH006: Protected branch update failed for refs/heads/main.",
        ] {
            assert!(matches!(
                protected(refused(stderr), "origin", "main"),
                GitError::BranchProtected { .. }
            ));
        }
        assert!(matches!(
            protected(
                refused("! [rejected] main -> main (fetch first)"),
                "origin",
                "main"
            ),
            GitError::CommandFailed { .. }
        ));
    }
}
