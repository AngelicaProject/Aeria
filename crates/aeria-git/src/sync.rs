//! Fetch, integrate, and push.
//!
//! Integration merges one or more upstream references atomically: either
//! every merge succeeds and the caller accepts the result, or the branch is
//! reset to where it started. Textual conflicts in unit shards are merged per
//! translation unit; only real same-unit conflicts are reported.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use aeria_core::TranslationUnitId;
use aeria_workspace::encode_unit_shard;

use crate::GitError;
use crate::collaboration::CollaborationPolicy;
use crate::merge::{ConflictResolution, merge_shard};
use crate::repository::{GitRepository, strip_prefix};
use crate::semantic::is_shard_path;

/// Commit message for managed changes written while accepting an
/// integration, such as reconciling merged units with the current source.
pub const RECONCILE_MESSAGE: &str = "Reconcile translations with the current game source";

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

    /// Integrates already fetched commits into the current branch: its
    /// upstream and, on a contribution branch under the pull-request policy,
    /// the remote main branch.
    ///
    /// Translation changes must be checkpointed first. Shards that conflict
    /// textually are merged per translation unit. Same-unit conflicts are
    /// resolved only by an explicit entry in `resolutions`; otherwise the
    /// merge is aborted and they are returned as
    /// [`GitError::TranslationConflicts`]. Once everything merged, `accept`
    /// must validate the resulting project; on failure the branch is reset
    /// to its starting commit. `accept` may rewrite Aeria-managed files to
    /// reconcile merged units with the current source; such changes are
    /// committed with [`RECONCILE_MESSAGE`].
    ///
    /// # Errors
    ///
    /// Returns [`GitError::TranslationConflicts`], [`GitError::MergeConflict`],
    /// [`GitError::IncomingRejected`], [`GitError::UncommittedTranslations`],
    /// or another typed Git error. The branch is unchanged after an error.
    pub fn integrate<F>(
        &self,
        resolutions: &BTreeMap<TranslationUnitId, ConflictResolution>,
        accept: F,
    ) -> Result<IntegrateOutcome, GitError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let branch = self.require_branch()?;
        self.require_clean_translations()?;

        let mut sources = Vec::new();
        if let Some(upstream) = self.upstream(&branch)? {
            sources.push(upstream);
        }
        let settings = self.collaboration()?;
        if let (CollaborationPolicy::PullRequest, Some(main)) =
            (settings.policy, &settings.main_branch)
            && main != &branch
        {
            let remote = self.sync_remote(&branch)?;
            let main_ref = format!("{remote}/{main}");
            if self
                .verify_ref(&format!("refs/remotes/{main_ref}"))?
                .is_some()
                && !sources.contains(&main_ref)
            {
                sources.push(main_ref);
            }
        }
        if sources.is_empty() {
            return Ok(IntegrateOutcome::UpToDate);
        }
        let Some(before) = self.head()? else {
            return Err(GitError::UnbornHead);
        };

        let mut outcome = IntegrateOutcome::UpToDate;
        for source in &sources {
            match self.merge_from(source, resolutions) {
                Ok(step) => outcome = outcome.max(step),
                Err(error) => {
                    self.reset_to(&before)?;
                    return Err(error);
                }
            }
        }
        if outcome.changed_working_tree() {
            if let Err(reason) = accept() {
                self.reset_to(&before)?;
                return Err(GitError::IncomingRejected { reason });
            }
            // Acceptance may reconcile the merged units with the current
            // source. Committing that keeps the pushed history consistent.
            if let Err(error) = self.commit_integration_changes(RECONCILE_MESSAGE) {
                self.reset_to(&before)?;
                return Err(error);
            }
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

    fn reset_to(&self, commit: &str) -> Result<(), GitError> {
        if self.merge_in_progress()? {
            self.run(&["merge", "--abort"])?;
        }
        if self.head()?.as_deref() != Some(commit) {
            self.run(&["reset", "--merge", "--quiet", commit])?;
        }
        Ok(())
    }

    fn merge_from(
        &self,
        source: &str,
        resolutions: &BTreeMap<TranslationUnitId, ConflictResolution>,
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
        let other: Vec<String> = conflicted
            .iter()
            .filter(|path| !is_shard_path(path))
            .cloned()
            .collect();
        if !other.is_empty() {
            return Err(GitError::MergeConflict { files: other });
        }

        let mut conflicts = Vec::new();
        let mut merged = Vec::new();
        for path in &conflicted {
            let top = self.top_level_path(path);
            let stages = self.read_blobs(&[
                format!(":1:{top}"),
                format!(":2:{top}"),
                format!(":3:{top}"),
            ])?;
            let [base, ours, theirs] = stages.as_slice() else {
                return Err(GitError::Parse {
                    message: format!("missing merge stages for {path}"),
                });
            };
            let shard = merge_shard(
                path,
                base.as_deref(),
                ours.as_deref(),
                theirs.as_deref(),
                resolutions,
            )?;
            conflicts.extend(shard.conflicts);
            merged.push((path, shard.units));
        }
        if !conflicts.is_empty() {
            return Err(GitError::TranslationConflicts { conflicts });
        }

        for (path, units) in merged {
            let full = self.root().join(path);
            if units.is_empty() {
                match fs::remove_file(&full) {
                    Ok(()) => {}
                    Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => {
                        return Err(GitError::Io {
                            operation: "remove merged unit shard",
                            path: full,
                            source,
                        });
                    }
                }
            } else {
                let bytes = encode_unit_shard(&units, std::path::Path::new(path))?;
                fs::write(&full, bytes).map_err(|source| GitError::Io {
                    operation: "write merged unit shard",
                    path: full.clone(),
                    source,
                })?;
            }
            self.run(&["add", "--all", "--", path])?;
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
            self.run(&["push", "--quiet", &remote, &refspec])?;
        } else {
            let refspec = format!("HEAD:refs/heads/{branch}");
            self.run(&["push", "--quiet", "--set-upstream", &remote, &refspec])?;
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

    fn conflicted_files(&self) -> Result<Vec<String>, GitError> {
        let text = self.run_text(&["diff", "--name-only", "--diff-filter=U", "-z"])?;
        let files: BTreeSet<String> = text
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(|path| strip_prefix(path, self.prefix()))
            .collect();
        Ok(files.into_iter().collect())
    }
}
