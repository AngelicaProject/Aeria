//! Branches and the pull-request contribution workflow.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::GitError;
use crate::collaboration::CollaborationPolicy;
use crate::repository::GitRepository;
use crate::sync::IntegrateOutcome;

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

/// The state of the current contribution under the pull-request policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContributionStatus {
    /// The branch contributions are reviewed into.
    pub main_branch: String,
    /// The current contribution branch, or `None` while on the main branch.
    pub branch: Option<String>,
    /// Whether the contribution branch has been published.
    pub published: bool,
    /// Commits on the contribution branch that the remote main branch does
    /// not contain yet. Zero after a merge-commit or fast-forward review
    /// merge; squash merges keep this non-zero.
    pub unmerged_commits: u32,
}

/// The result of finishing a contribution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishOutcome {
    pub integration: IntegrateOutcome,
    /// The finished contribution branch, when it was deleted locally because
    /// Git confirmed it was merged.
    pub deleted_branch: Option<String>,
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

    /// Returns the contribution state under the pull-request policy, or
    /// `None` under the direct policy.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid settings or a Git failure.
    pub fn contribution_status(&self) -> Result<Option<ContributionStatus>, GitError> {
        let settings = self.collaboration()?;
        let (CollaborationPolicy::PullRequest, Some(main_branch)) =
            (settings.policy, settings.main_branch)
        else {
            return Ok(None);
        };
        let Some(branch) = self.current_branch()? else {
            return Ok(None);
        };
        if branch == main_branch {
            return Ok(Some(ContributionStatus {
                main_branch,
                branch: None,
                published: false,
                unmerged_commits: 0,
            }));
        }
        let published = self.upstream(&branch)?.is_some();
        let remote_main = match self.sync_remote(&branch) {
            Ok(remote) => Some(format!("{remote}/{main_branch}")).filter(|reference| {
                self.verify_ref(&format!("refs/remotes/{reference}"))
                    .is_ok_and(|found| found.is_some())
            }),
            Err(GitError::NoRemote) => None,
            Err(error) => return Err(error),
        };
        let base = remote_main.unwrap_or_else(|| main_branch.clone());
        let unmerged_commits = if self.verify_ref(&base)?.is_some() {
            self.ahead_behind(&base)?.0
        } else {
            0
        };
        Ok(Some(ContributionStatus {
            main_branch,
            branch: Some(branch),
            published,
            unmerged_commits,
        }))
    }

    /// Returns to the main branch after a contribution was reviewed:
    /// switches to it, integrates its upstream, and deletes the contribution
    /// branch when Git confirms it is merged. `accept` validates the
    /// resulting project; on failure the contribution branch is restored.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidSettings`] outside the pull-request policy,
    /// [`GitError::UncommittedTranslations`], [`GitError::IncomingRejected`],
    /// or another typed Git error.
    pub fn finish_contribution<F>(&self, accept: F) -> Result<FinishOutcome, GitError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let settings = self.collaboration()?;
        let (CollaborationPolicy::PullRequest, Some(main)) =
            (settings.policy, settings.main_branch)
        else {
            return Err(GitError::InvalidSettings {
                reason: "contributions are used only with the pull-request policy".to_owned(),
            });
        };
        self.require_clean_translations()?;
        let contribution = self.require_branch()?;
        if contribution == main {
            return Ok(FinishOutcome {
                integration: IntegrateOutcome::UpToDate,
                deleted_branch: None,
            });
        }
        self.run(&["switch", "--quiet", &main])?;
        let integration = match self.upstream(&main)? {
            Some(upstream) if self.ahead_behind(&upstream)?.1 > 0 => {
                match self.run(&["merge", "--ff-only", "--quiet", &upstream]) {
                    Ok(_) => IntegrateOutcome::FastForward,
                    Err(error) => {
                        self.run(&["switch", "--quiet", &contribution])?;
                        return Err(error);
                    }
                }
            }
            _ => IntegrateOutcome::UpToDate,
        };
        if let Err(reason) = accept() {
            self.run(&["switch", "--quiet", &contribution])?;
            return Err(GitError::IncomingRejected { reason });
        }
        let deleted = self
            .output(&["branch", "--quiet", "-d", &contribution])?
            .status
            .success();
        Ok(FinishOutcome {
            integration,
            deleted_branch: deleted.then_some(contribution),
        })
    }

    /// Returns a new branch name such as `translations/ada-20260923-141502`,
    /// derived from the translator name and the current UTC time.
    pub(crate) fn new_contribution_branch_name(&self) -> Result<String, GitError> {
        let identity = self.identity()?;
        let slug = slug(identity.name.as_deref().unwrap_or_default())
            .or_else(|| {
                identity
                    .email
                    .as_deref()
                    .and_then(|email| email.split('@').next())
                    .and_then(slug)
            })
            .unwrap_or_else(|| "translator".to_owned());
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let base = format!("translations/{slug}-{}", utc_stamp(seconds));
        let mut name = base.clone();
        let mut suffix = 2;
        while self.verify_ref(&format!("refs/heads/{name}"))?.is_some() {
            name = format!("{base}-{suffix}");
            suffix += 1;
        }
        Ok(name)
    }
}

/// Lowercase ASCII letters and digits separated by single hyphens.
fn slug(text: &str) -> Option<String> {
    let mut slug = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 32 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-').to_owned();
    (!slug.is_empty()).then_some(slug)
}

/// Formats Unix seconds as `YYYYMMDD-HHMMSS` in UTC.
fn utc_stamp(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let second_of_day = seconds % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        second_of_day / 3_600,
        second_of_day % 3_600 / 60,
        second_of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_slugs_are_ascii_and_bounded() {
        assert_eq!(slug("Ada Lovelace").as_deref(), Some("ada-lovelace"));
        assert_eq!(slug("  --Grace__H. "), Some("grace-h".to_owned()));
        assert_eq!(slug("Анна"), None);
    }

    #[test]
    fn utc_stamps_use_the_civil_calendar() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(951_782_400), "20000229-000000");
        assert_eq!(utc_stamp(1_790_172_902), "20260923-141502");
    }
}
