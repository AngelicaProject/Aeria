//! `write` and `check`: translations from an agent, checked before anything
//! is stored.

use std::fmt::Write as _;

use aeria_core::ReviewState;
use aeria_knowledge::Knowledge;
use aeria_knowledge::rules::machine_phrasing;
use aeria_workspace::AssistedExpectation;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Output;
use super::project::{Address, Author, Env, Project, current, other_languages, source_of};
use crate::sync::WriteLock;

/// One translation to write.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub address: Address,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonEntry {
    at: String,
    text: String,
}

/// Reads translations: JSON Lines (`{"at": "<address>", "text": "…"}` per
/// line) when the input starts with `{`, otherwise blocks of an
/// `@<address>` line followed by the translation. Anything after the address
/// on its line, such as what `aeria read` prints there, is ignored, as are
/// blank lines and lines starting with `#` between blocks.
pub(crate) fn parse_entries(input: &str) -> Result<Vec<Entry>, String> {
    let input = input.trim_start_matches('\u{feff}');
    if input.trim_start().starts_with('{') {
        return input
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(number, line)| {
                let entry: JsonEntry = serde_json::from_str(line)
                    .map_err(|error| format!("line {}: {error}", number + 1))?;
                Ok(Entry {
                    address: Address::parse(&entry.at)
                        .map_err(|error| format!("line {}: {error}", number + 1))?,
                    text: entry.text,
                })
            })
            .collect();
    }
    let mut entries: Vec<(Address, Vec<&str>)> = Vec::new();
    for (number, line) in input.lines().enumerate() {
        if let Some(rest) = line.strip_prefix('@') {
            let address = rest
                .split(|character: char| character.is_whitespace() || character == '·')
                .next()
                .unwrap_or_default();
            let address =
                Address::parse(address).map_err(|error| format!("line {}: {error}", number + 1))?;
            entries.push((address, Vec::new()));
        } else if let Some((_, text)) = entries.last_mut() {
            text.push(line);
        } else if !line.trim().is_empty() && !line.starts_with('#') {
            return Err(format!(
                "line {}: a translation must follow an @<address> line",
                number + 1
            ));
        }
    }
    Ok(entries
        .into_iter()
        .map(|(address, lines)| Entry {
            address,
            text: lines.join("\n").trim_matches('\n').trim_end().to_owned(),
        })
        .collect())
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
    /// Check only; nothing is written.
    pub dry_run: bool,
    /// Mark written translations as needing review instead of drafts.
    pub needs_review: bool,
}

#[allow(clippy::too_many_lines)] // one pass over the entries
pub(crate) fn write(
    start: &std::path::Path,
    env: &Env,
    entries: &[Entry],
    options: &WriteOptions,
    out: &mut Output,
) -> Result<bool, String> {
    // A write opens the project under the lock, so it starts from the
    // workspace every earlier writer left.
    let (mut project, _lock) = if options.dry_run {
        (Project::open(start, env)?, None)
    } else {
        let lock = env
            .project_files(start)?
            .as_ref()
            .map(WriteLock::acquire)
            .transpose()
            .map_err(|error| format!("the project's write lock cannot be taken: {error}"))?;
        (Project::open(start, env)?, lock)
    };
    let target_language = project
        .target_language()
        .ok_or("the project has no target language yet; set it in Aeria's project settings")?;
    let knowledge = Knowledge::load(project.root());
    let mut ledger = project.ledger();
    let mut outcomes: Vec<(String, Outcome)> = Vec::with_capacity(entries.len());
    let mut written: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let address = entry.address.to_string();
        let source = match source_of(&project, &entry.address) {
            Ok(source) => source,
            Err(reason) => {
                outcomes.push((
                    address,
                    Outcome::Rejected {
                        reasons: vec![reason],
                    },
                ));
                continue;
            }
        };
        let now = current(&project.session, ledger.as_ref(), &entry.address);
        if let Some(now) = &now {
            if now.target == entry.text {
                outcomes.push((address, Outcome::Unchanged));
                continue;
            }
            if now.review == ReviewState::Reviewed {
                outcomes.push((
                    address,
                    Outcome::Skipped {
                        reason: "reviewed; a person confirmed this translation".to_owned(),
                    },
                ));
                continue;
            }
            if now.author == Author::Person {
                outcomes.push((address, Outcome::Skipped {
                    reason: "a person wrote or changed this translation; tell the user instead of replacing it".to_owned(),
                }));
                continue;
            }
        }
        let (reasons, advice) = check_text(&project, &knowledge, &target_language, entry, &source);
        if !reasons.is_empty() {
            outcomes.push((address, Outcome::Rejected { reasons }));
            continue;
        }
        if options.dry_run {
            outcomes.push((address, Outcome::Ok { advice }));
            continue;
        }
        let expected = AssistedExpectation {
            target: now.as_ref().map(|now| now.target.clone()),
            review_state: now.as_ref().map(|now| now.review),
        };
        let binding = entry.address.binding();
        let result = project
            .session
            .set_assisted_target(&binding, &entry.text, &expected, false)
            .map_err(|error| error.to_string())
            .and_then(|id| {
                if options.needs_review {
                    project
                        .session
                        .set_review_state(id, ReviewState::NeedsReview)
                        .map_err(|error| error.to_string())
                } else {
                    Ok(())
                }
            });
        match result {
            Ok(()) => {
                written.push((address.clone(), entry.text.clone()));
                // Later entries in this batch see the text as the agent's.
                if let Some(ledger) = ledger.as_mut() {
                    let _ = ledger.record(&[(address.clone(), entry.text.clone())]);
                }
                outcomes.push((address, Outcome::Ok { advice }));
            }
            Err(reason) => outcomes.push((
                address,
                Outcome::Rejected {
                    reasons: vec![reason],
                },
            )),
        }
    }
    if !written.is_empty()
        && let Some(files) = &project.files
    {
        files.touch_stamp().map_err(|error| {
            format!("the translations were written, but open Aeria windows were not told: {error}")
        })?;
    }
    let rejected = outcomes
        .iter()
        .filter(|(_, outcome)| matches!(outcome, Outcome::Rejected { .. }))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|(_, outcome)| matches!(outcome, Outcome::Skipped { .. }))
        .count();
    if out.json {
        out.json_value(&json!({
            "written": if options.dry_run { 0 } else { written.len() },
            "rejected": rejected,
            "skipped": skipped,
            "results": outcomes
                .iter()
                .map(|(address, outcome)| json!({ "at": address, "result": outcome }))
                .collect::<Vec<_>>(),
        }));
        return Ok(rejected == 0);
    }
    for (address, outcome) in &outcomes {
        match outcome {
            Outcome::Ok { advice } => {
                let verb = if options.dry_run { "ok" } else { "written" };
                if advice.is_empty() {
                    let _ = writeln!(out.text, "{verb} @{address}");
                } else {
                    let _ = writeln!(
                        out.text,
                        "{verb} @{address} — advice: {}",
                        advice.join("; ")
                    );
                }
            }
            Outcome::Rejected { reasons } => {
                let _ = writeln!(out.text, "REJECTED @{address}: {}", reasons.join("; "));
            }
            Outcome::Skipped { reason } => {
                let _ = writeln!(out.text, "skipped @{address}: {reason}");
            }
            Outcome::Unchanged => {
                let _ = writeln!(out.text, "unchanged @{address}");
            }
        }
    }
    let ok = outcomes
        .iter()
        .filter(|(_, outcome)| matches!(outcome, Outcome::Ok { .. }))
        .count();
    let _ = writeln!(
        out.text,
        "{} {ok} · rejected {rejected} · skipped {skipped}{}",
        if options.dry_run {
            "would write"
        } else {
            "written"
        },
        if rejected > 0 {
            " — fix the rejected lines and write them again"
        } else {
            ""
        }
    );
    Ok(rejected == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_and_json_lines_are_read() {
        let entries = parse_entries(
            "# a comment\n@Addon:1:0:1 · text · untranslated\nОК\n\n@quest/000/A:3:0:1\nПервая <br>вторая\n",
        )
        .expect("blocks");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].address.to_string(), "Addon:1:0:1");
        assert_eq!(entries[0].text, "ОК");
        assert_eq!(entries[1].text, "Первая <br>вторая");

        let entries = parse_entries(
            "{\"at\": \"Addon:1:0:1\", \"text\": \"ОК\"}\n\n{\"at\": \"@Addon:2:0:1\", \"text\": \"Отмена\"}\n",
        )
        .expect("json");
        assert_eq!(entries[1].address.row, 2);
        assert!(parse_entries("ОК\n").is_err());
        assert!(parse_entries("@Addon:1\nОК").is_err());
    }
}
