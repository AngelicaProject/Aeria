//! Saving translations: each is checked before anything is stored, and
//! only an agent's own translations are replaced.

use aeria_core::ReviewState;
use aeria_knowledge::Knowledge;
use aeria_knowledge::rules::machine_phrasing;
use aeria_workspace::{AssistedExpectation, AssistedWrite, AssistedWriteError};
use serde::Serialize;

use super::Output;
use super::project::{Address, Author, Project, current, other_languages, source_of};

/// One translation to write.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub address: Address,
    pub text: String,
}

/// What happened to one translation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub(crate) enum Outcome {
    /// Written, or would be written by `check`.
    Ok { advice: Vec<String> },
    /// The translation is wrong and was not written.
    Rejected { reasons: Vec<String> },
    /// The string is not an agent's to change.
    Skipped { reason: String },
    /// The translation passed its checks but could not be saved, for
    /// example because the disk refused the write; writing it again may
    /// succeed.
    Failed { reason: String },
    /// The string already has this translation.
    Unchanged,
}

/// Russian forms that write both genders at once, such as `готов(а)`.
const BOTH_GENDERS: [&str; 6] = ["(а)", "(ла)", "(ая)", "(на)", "(ен)", "(ой)"];

/// Checks one translation against its source and the knowledge. Rejections
/// are what must be fixed; advice is written with it.
fn check_text(
    project: &Project,
    knowledge: &Knowledge,
    target_language: &str,
    entry: &Entry,
    source: &str,
) -> (Vec<String>, Vec<String>) {
    let mut reasons = Vec::new();
    let mut advice = Vec::new();
    let text = &entry.text;
    if text.trim().is_empty() {
        reasons.push("the translation is empty".to_owned());
        return (reasons, advice);
    }
    if text.contains('\n') && !source.contains('\n') {
        reasons.push("the translation has a line break the source does not; write one translation per line and <br> where the game breaks the line".to_owned());
    }
    if let Err(errors) = aeria_se::check_assisted_structure(source, text) {
        reasons.extend(errors.into_iter().map(|error| error.message));
    }
    reasons.extend(knowledge.terms.forbidden_in(source, text));
    let russian = target_language.eq_ignore_ascii_case("ru");
    if russian
        && let Some(form) = BOTH_GENDERS
            .iter()
            .find(|form| text.contains(*form) && !source.contains(*form))
    {
        reasons.push(format!(
            "{form} writes both genders at once; use a condition on $gn4 with the feminine form first, or a phrasing that shows no gender"
        ));
    }
    advice.extend(knowledge.terms.missing_in(source, text));
    if source.contains("$gn4") && !text.contains("$gn4") {
        advice.push("the source varies with the player character's gender and the translation does not; make sure nothing in it agrees with the player character's gender".to_owned());
    } else if !source.contains("$gn4") && !text.contains("$gn4") && russian {
        let varies = other_languages(project, &entry.address)
            .iter()
            .any(|(code, text)| matches!(code.as_str(), "fr" | "de") && text.contains("$gn4"));
        if varies {
            advice.push("the French or German line varies with the player character's gender; check whether a word about the player character needs a condition on $gn4".to_owned());
        }
    }
    let phrasing = machine_phrasing(target_language, text);
    if !phrasing.is_empty() {
        advice.push(format!("reads machine-written: {}", phrasing.join(", ")));
    }
    (reasons, advice)
}

pub(crate) struct WriteOptions {
    /// Mark written translations as needing review instead of drafts.
    pub needs_review: bool,
}

/// A translation that passed its checks, ready to be written.
/// What judging one translation gave: ready to write, or an outcome.
type Judged = Vec<(String, Result<Pending, Outcome>)>;

struct Pending {
    entry: usize,
    advice: Vec<String>,
    expected: AssistedExpectation,
}

/// Judges translations without writing: each either passes with its advice
/// or ends with an outcome. A later translation of the same string replaces
/// an earlier one in the batch.
fn judge(project: &Project, entries: &[Entry]) -> Result<Judged, String> {
    let target_language = project
        .target_language()
        .ok_or("the project has no target language yet; set it in Aeria's project settings")?;
    let knowledge = project.knowledge();
    let ledger = project.ledger();
    let last: std::collections::HashMap<&Address, usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (&entry.address, index))
        .collect();
    let mut judged = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let address = entry.address.to_string();
        if last.get(&entry.address) != Some(&index) {
            continue;
        }
        let source = match source_of(project, &entry.address) {
            Ok(source) => source,
            Err(reason) => {
                judged.push((
                    address,
                    Err(Outcome::Rejected {
                        reasons: vec![reason],
                    }),
                ));
                continue;
            }
        };
        let now = current(&project.session, ledger.as_ref(), &entry.address);
        if let Some(now) = &now {
            let skip = if now.target == entry.text {
                Some(Outcome::Unchanged)
            } else if now.review == ReviewState::Reviewed {
                Some(Outcome::Skipped {
                    reason: "reviewed; a person confirmed this translation".to_owned(),
                })
            } else if now.author == Author::Person {
                Some(Outcome::Skipped {
                    reason: "a person wrote or changed this translation; tell the user instead of replacing it".to_owned(),
                })
            } else {
                None
            };
            if let Some(outcome) = skip {
                judged.push((address, Err(outcome)));
                continue;
            }
        }
        let (reasons, advice) = check_text(project, &knowledge, &target_language, entry, &source);
        if !reasons.is_empty() {
            judged.push((address, Err(Outcome::Rejected { reasons })));
            continue;
        }
        judged.push((
            address,
            Ok(Pending {
                entry: index,
                advice,
                expected: AssistedExpectation {
                    target: now.as_ref().map(|now| now.target.clone()),
                    review_state: now.as_ref().map(|now| now.review),
                },
            }),
        ));
    }
    Ok(judged)
}

/// Writes translations that pass their checks and returns the outcome of
/// each, by address, and how many were written.
pub(crate) fn apply(
    project: &mut Project,
    entries: &[Entry],
    options: &WriteOptions,
    out: &mut Output,
) -> Result<(Vec<(String, Outcome)>, usize), String> {
    let judged = judge(project, entries)?;
    let mut ledger = project.ledger();
    // Every translation that passed is written in one batch, so each shard
    // is published once.
    let mut outcomes: Vec<(String, Option<Outcome>)> = Vec::with_capacity(judged.len());
    let mut batch = Vec::new();
    let mut pending_at = Vec::new();
    for (address, judged) in judged {
        match judged {
            Ok(pending) => {
                let entry = &entries[pending.entry];
                batch.push(AssistedWrite {
                    source_binding: entry.address.binding(),
                    target_macro: entry.text.clone(),
                    expected: pending.expected.clone(),
                    review_state: options.needs_review.then_some(ReviewState::NeedsReview),
                });
                pending_at.push((outcomes.len(), pending));
                outcomes.push((address, None));
            }
            Err(outcome) => outcomes.push((address, Some(outcome))),
        }
    }
    let results = project.session.set_assisted_targets(&batch, false);
    let mut written: Vec<(String, String)> = Vec::new();
    for ((index, pending), result) in pending_at.into_iter().zip(results) {
        let outcome = match result {
            Ok(_) => {
                written.push((
                    outcomes[index].0.clone(),
                    entries[pending.entry].text.clone(),
                ));
                Outcome::Ok {
                    advice: pending.advice,
                }
            }
            Err(AssistedWriteError::Structure { messages }) => {
                Outcome::Rejected { reasons: messages }
            }
            Err(other) => Outcome::Failed {
                reason: other.to_string(),
            },
        };
        outcomes[index].1 = Some(outcome);
    }
    let outcomes: Vec<(String, Outcome)> = outcomes
        .into_iter()
        .filter_map(|(address, outcome)| outcome.map(|outcome| (address, outcome)))
        .collect();
    if let Some(ledger) = ledger.as_mut()
        && let Err(error) = ledger.record(&written)
    {
        out.warn(&format!(
            "the translations were written, but the record of which are agents' failed ({error}); they may count as a person's"
        ));
    }
    // The translations are written either way; an open Aeria window then
    // shows them once it reopens the project.
    if !written.is_empty()
        && let Some(files) = &project.files
        && let Err(error) = files.touch_stamp()
    {
        out.warn(&format!(
            "the translations were written, but an open Aeria window may show them only after it reopens the project: {error}"
        ));
    }
    Ok((outcomes, written.len()))
}
