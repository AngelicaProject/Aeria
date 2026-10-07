//! Whether a GitHub repository guards its main branch and its pack release
//! tags as Aeria recommends, read without signing in, and the rulesets to
//! import on GitHub where it does not.
//!
//! The main branch takes changes only through pull requests that merge
//! (merge commits keep each branch's history, which Aeria's per-string
//! merge reads) once the Aeria Guard check passed, and it can be neither
//! deleted nor force-pushed. Pack release tags, which deliver packs to
//! players through the feed, can be created only by maintainers and
//! administrators and never moved or deleted.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::github::{
    GitHubClient, GitHubRepository, PublishError, RELEASE_TAG_PREFIX, parse, send,
};

/// The name of the job in the Aeria Guard workflow, which GitHub reports as
/// its status check.
pub const GUARD_CHECK: &str = "Aeria Guard";

/// GitHub's ids of repository roles in a ruleset's bypass list.
const MAINTAIN_ROLE: u64 = 2;
const ADMIN_ROLE: u64 = 5;

/// A rule the main branch should have.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchRule {
    /// The branch cannot be deleted.
    Deletion,
    /// The branch cannot be force-pushed.
    ForcePush,
    /// Changes come through pull requests.
    PullRequest,
    /// Pull requests merge only once the Aeria Guard check passed.
    GuardCheck,
}

/// A rule pack release tags should have.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TagRule {
    /// Only maintainers and administrators create them.
    Creation,
    /// They cannot be moved.
    Update,
    /// They cannot be deleted.
    Deletion,
}

/// How far a repository has the rules Aeria recommends.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum RuleState<Rule> {
    Protected,
    /// Some rules are in place; `missing` are not.
    Partial {
        missing: Vec<Rule>,
    },
    Missing,
    /// GitHub did not tell: the repository is private (rules are read
    /// without signing in), the branch is protected by classic branch
    /// protection, or GitHub could not be reached.
    Unknown,
}

impl<Rule: Copy + PartialEq> RuleState<Rule> {
    fn of(wanted: &[Rule], present: &[Rule]) -> Self {
        let missing: Vec<Rule> = wanted
            .iter()
            .copied()
            .filter(|rule| !present.contains(rule))
            .collect();
        if missing.is_empty() {
            Self::Protected
        } else if missing.len() == wanted.len() {
            Self::Missing
        } else {
            Self::Partial { missing }
        }
    }
}

/// The protection of a repository's main branch and pack release tags.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Protection {
    pub branch: RuleState<BranchRule>,
    pub tags: RuleState<TagRule>,
}

const BRANCH_RULES: [BranchRule; 4] = [
    BranchRule::Deletion,
    BranchRule::ForcePush,
    BranchRule::PullRequest,
    BranchRule::GuardCheck,
];
const TAG_RULES: [TagRule; 3] = [TagRule::Creation, TagRule::Update, TagRule::Deletion];

#[derive(Deserialize)]
struct RuleJson {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    parameters: Value,
}

#[derive(Deserialize)]
struct RulesetSummary {
    id: u64,
    #[serde(default)]
    target: String,
}

#[derive(Deserialize)]
struct RulesetJson {
    #[serde(default)]
    enforcement: String,
    #[serde(default)]
    conditions: Value,
    #[serde(default)]
    rules: Vec<RuleJson>,
}

#[derive(Deserialize)]
struct BranchJson {
    #[serde(default)]
    protected: bool,
}

/// The branch rules among rules GitHub applies to a branch.
fn branch_rules(rules: &[RuleJson]) -> Vec<BranchRule> {
    rules
        .iter()
        .filter_map(|rule| match rule.kind.as_str() {
            "deletion" => Some(BranchRule::Deletion),
            "non_fast_forward" => Some(BranchRule::ForcePush),
            "pull_request" => Some(BranchRule::PullRequest),
            "required_status_checks" => rule.parameters["required_status_checks"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|check| check["context"] == GUARD_CHECK)
                .then_some(BranchRule::GuardCheck),
            _ => None,
        })
        .collect()
}

/// Whether a ruleset's ref pattern covers `reference`: `~ALL`, or a pattern
/// where `**/` matches any folders, none included, `**` anything, and `*`
/// anything but `/`.
fn pattern_matches(pattern: &str, reference: &str) -> bool {
    fn matches(pattern: &[u8], text: &[u8]) -> bool {
        match pattern {
            [] => text.is_empty(),
            [b'*', b'*', b'/', rest @ ..] => {
                matches(rest, text)
                    || (0..text.len())
                        .filter(|index| text[*index] == b'/')
                        .any(|index| matches(rest, &text[index + 1..]))
            }
            [b'*', b'*', rest @ ..] => (0..=text.len()).any(|skip| matches(rest, &text[skip..])),
            [b'*', rest @ ..] => (0..=text.len())
                .take_while(|skip| *skip == 0 || text[skip - 1] != b'/')
                .any(|skip| matches(rest, &text[skip..])),
            [first, rest @ ..] => text.first() == Some(first) && matches(rest, &text[1..]),
        }
    }
    pattern == "~ALL" || matches(pattern.as_bytes(), reference.as_bytes())
}

/// The tag rules of an active tag ruleset that covers pack release tags.
fn tag_rules(ruleset: &RulesetJson) -> Vec<TagRule> {
    let sample = format!("refs/tags/{RELEASE_TAG_PREFIX}1.0.0");
    let patterns = |key: &str| -> Vec<String> {
        ruleset.conditions["ref_name"][key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|pattern| pattern.as_str().map(str::to_owned))
            .collect()
    };
    let covered = patterns("include")
        .iter()
        .any(|pattern| pattern_matches(pattern, &sample))
        && !patterns("exclude")
            .iter()
            .any(|pattern| pattern_matches(pattern, &sample));
    if ruleset.enforcement != "active" || !covered {
        return Vec::new();
    }
    ruleset
        .rules
        .iter()
        .filter_map(|rule| match rule.kind.as_str() {
            "creation" => Some(TagRule::Creation),
            "update" => Some(TagRule::Update),
            "deletion" => Some(TagRule::Deletion),
            _ => None,
        })
        .collect()
}

impl GitHubClient {
    async fn get_anonymous<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
    ) -> Result<T, PublishError> {
        parse(
            send(
                self.http
                    .request(Method::GET, url)
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", crate::github::API_VERSION),
            )
            .await?,
        )
        .await
    }

    /// How the repository protects `branch` and its pack release tags, read
    /// without signing in.
    pub async fn protection(&self, repository: &GitHubRepository, branch: &str) -> Protection {
        let base = format!(
            "{}/repos/{}/{}",
            self.api_base, repository.owner, repository.name
        );
        let branch_path: String = branch
            .chars()
            .map(|character| match character {
                '%' | '#' | '?' | ' ' => format!("%{:02X}", character as u32),
                other => other.to_string(),
            })
            .collect();
        let branch_state = match self
            .get_anonymous::<Vec<RuleJson>>(&format!("{base}/rules/branches/{branch_path}"))
            .await
        {
            Ok(rules) if rules.is_empty() => {
                // Classic branch protection is not among the rules, and its
                // settings are not readable without signing in.
                match self
                    .get_anonymous::<BranchJson>(&format!("{base}/branches/{branch_path}"))
                    .await
                {
                    Ok(branch) if branch.protected => RuleState::Unknown,
                    Ok(_) => RuleState::Missing,
                    Err(_) => RuleState::Unknown,
                }
            }
            Ok(rules) => RuleState::of(&BRANCH_RULES, &branch_rules(&rules)),
            Err(_) => RuleState::Unknown,
        };
        let tag_state = match self
            .get_anonymous::<Vec<RulesetSummary>>(&format!(
                "{base}/rulesets?includes_parents=true&per_page=100"
            ))
            .await
        {
            Ok(rulesets) => {
                let mut present = Vec::new();
                let mut failed = false;
                for summary in rulesets.iter().filter(|ruleset| ruleset.target == "tag") {
                    match self
                        .get_anonymous::<RulesetJson>(&format!("{base}/rulesets/{}", summary.id))
                        .await
                    {
                        Ok(ruleset) => present.extend(tag_rules(&ruleset)),
                        Err(_) => failed = true,
                    }
                }
                match RuleState::of(&TAG_RULES, &present) {
                    RuleState::Protected => RuleState::Protected,
                    _ if failed => RuleState::Unknown,
                    state => state,
                }
            }
            Err(_) => RuleState::Unknown,
        };
        Protection {
            branch: branch_state,
            tags: tag_state,
        }
    }
}

/// The ruleset for the default branch, to import on GitHub (Settings →
/// Rules → Rulesets → New ruleset → Import a ruleset).
#[must_use]
pub fn branch_ruleset() -> Value {
    json!({
        "name": "Main branch",
        "target": "branch",
        "enforcement": "active",
        "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
        "bypass_actors": [],
        "rules": [
            { "type": "deletion" },
            { "type": "non_fast_forward" },
            {
                "type": "pull_request",
                "parameters": {
                    "required_approving_review_count": 0,
                    "dismiss_stale_reviews_on_push": false,
                    "require_code_owner_review": false,
                    "require_last_push_approval": false,
                    "required_review_thread_resolution": false,
                    "allowed_merge_methods": ["merge"]
                }
            },
            {
                "type": "required_status_checks",
                "parameters": {
                    "strict_required_status_checks_policy": true,
                    "do_not_enforce_on_create": false,
                    "required_status_checks": [{ "context": GUARD_CHECK }]
                }
            }
        ]
    })
}

/// The ruleset for pack release tags, to import on GitHub.
#[must_use]
pub fn tag_ruleset() -> Value {
    json!({
        "name": "Pack releases",
        "target": "tag",
        "enforcement": "active",
        "conditions": {
            "ref_name": { "include": [format!("refs/tags/{RELEASE_TAG_PREFIX}**/*")], "exclude": [] }
        },
        "bypass_actors": [
            { "actor_id": MAINTAIN_ROLE, "actor_type": "RepositoryRole", "bypass_mode": "always" },
            { "actor_id": ADMIN_ROLE, "actor_type": "RepositoryRole", "bypass_mode": "always" }
        ],
        "rules": [
            { "type": "creation" },
            { "type": "update" },
            { "type": "deletion" }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(value: Value) -> Vec<RuleJson> {
        serde_json::from_value(value).expect("rules")
    }

    #[test]
    fn branch_rules_need_the_guard_check() {
        let present = branch_rules(&rules(json!([
            { "type": "deletion" },
            { "type": "non_fast_forward" },
            { "type": "pull_request", "parameters": {} },
            { "type": "required_status_checks", "parameters": { "required_status_checks": [{ "context": "ci" }] } }
        ])));
        assert_eq!(
            RuleState::of(&BRANCH_RULES, &present),
            RuleState::Partial {
                missing: vec![BranchRule::GuardCheck]
            }
        );
        let imported: Vec<RuleJson> = rules(branch_ruleset()["rules"].clone());
        assert_eq!(
            RuleState::of(&BRANCH_RULES, &branch_rules(&imported)),
            RuleState::Protected
        );
        assert_eq!(
            RuleState::<BranchRule>::of(&BRANCH_RULES, &[]),
            RuleState::Missing
        );
    }

    #[test]
    fn ref_patterns_match_as_github_does() {
        let tag = "refs/tags/harmonia/1.0.0";
        for pattern in [
            "~ALL",
            "refs/tags/harmonia/*",
            "refs/tags/harmonia/**/*",
            "refs/tags/**/*",
            "refs/tags/**",
        ] {
            assert!(pattern_matches(pattern, tag), "{pattern}");
        }
        for pattern in ["refs/tags/*", "refs/tags/v*", "refs/heads/**"] {
            assert!(!pattern_matches(pattern, tag), "{pattern}");
        }
    }

    #[test]
    fn the_imported_tag_ruleset_protects_pack_tags() {
        let ruleset: RulesetJson = serde_json::from_value(tag_ruleset()).expect("ruleset");
        assert_eq!(
            RuleState::of(&TAG_RULES, &tag_rules(&ruleset)),
            RuleState::Protected
        );
        let mut disabled = tag_ruleset();
        disabled["enforcement"] = json!("disabled");
        let disabled: RulesetJson = serde_json::from_value(disabled).expect("ruleset");
        assert!(tag_rules(&disabled).is_empty());
    }
}
