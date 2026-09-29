//! Learning: lessons from what critics found and what people changed, kept
//! only when they measurably help.
//!
//! When a job finishes, a mentor reads the critics' findings of the job,
//! the translations people changed after agents wrote them (reactions), and
//! the findings against the project knowledge itself. It writes lessons for
//! recurring problems, on trial, and corrects agent terms the findings show
//! to be wrong.
//!
//! A lesson on trial is evaluated: a few units the job translated are
//! localized again with the lesson in their knowledge, without writing, and
//! a judge compares them with what the job wrote, in both orders, with every
//! client language. Lessons that win are kept (`active`), lessons that lose
//! are dropped, and a draw leaves them on trial.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;

use crate::chat::Usage;
use crate::client::ProviderError;
use crate::guidance::GlossaryEntry;
use crate::jobs::Finding;
use crate::knowledge::{Domain, Lesson, LessonStatus};
use crate::localizer::{Caller, LocalizeOptions, Request, Role, UnitOfWork, localize};

/// Findings shown to the mentor, at most.
const MAX_MENTOR_FINDINGS: usize = 300;
/// Reactions shown to the mentor, at most.
const MAX_MENTOR_REACTIONS: usize = 80;
/// Lessons one mentor request writes, at most.
pub const MAX_NEW_LESSONS: usize = 5;

/// A translation a person changed after an agent wrote it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reaction {
    pub source: String,
    /// What the agent wrote.
    pub agent: String,
    /// What the person made of it.
    pub person: String,
}

/// What the mentor decided.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MentorReply {
    pub lessons: Vec<Lesson>,
    /// Agent terms to correct.
    pub terms: Vec<GlossaryEntry>,
}

/// A request for lessons and term corrections.
#[must_use]
pub fn mentor_request(
    target: &str,
    findings: &[Finding],
    reactions: &[Reaction],
    knowledge_findings: &[String],
    lessons: &[Lesson],
) -> Request {
    let mut text = String::new();
    let _ = writeln!(text, "Findings of the critics (major ones first):");
    let mut sorted: Vec<&Finding> = findings.iter().collect();
    sorted.sort_by_key(|finding| !finding.major);
    for finding in sorted.into_iter().take(MAX_MENTOR_FINDINGS) {
        let _ = writeln!(
            text,
            "- [{}{}] {} | source: {} | translation: {}",
            finding.role,
            if finding.major { ", major" } else { "" },
            finding.problem,
            finding.source,
            finding.target
        );
    }
    if !reactions.is_empty() {
        let _ = writeln!(
            text,
            "\nTranslations people changed after agents wrote them:"
        );
        for reaction in reactions.iter().take(MAX_MENTOR_REACTIONS) {
            let _ = writeln!(
                text,
                "- source: {} | agent: {} | person: {}",
                reaction.source, reaction.agent, reaction.person
            );
        }
    }
    if !knowledge_findings.is_empty() {
        let _ = writeln!(text, "\nFindings against the project knowledge itself:");
        for finding in knowledge_findings {
            let _ = writeln!(text, "- {finding}");
        }
    }
    let _ = writeln!(
        text,
        "\nLessons the project already has (do not repeat them; dropped ones did not help):"
    );
    for lesson in lessons {
        let _ = writeln!(
            text,
            "- {} ({}): {}",
            lesson.id,
            lesson.status.as_str(),
            lesson.text.replace('\n', " ")
        );
    }
    Request {
        role: Role::Mentor,
        part: None,
        system: format!(
            "You are the mentor of the agents that localize FINAL FANTASY XIV into {target}. You \
             turn recurring problems into lessons that keep the agents from repeating them."
        ),
        user: format!(
            "{text}\nWrite at most {MAX_NEW_LESSONS} new lessons, each for a problem that recurs in \
             several findings or that people corrected more than once; a single occurrence is not \
             a lesson. A lesson says in one or two sentences what to do instead, with a short \
             example in {target}, and names the kind of text it applies to when it applies to one: \
             general, journal, objective, system, dialogue, names, items, actions, interface, or \
             lore. What people changed weighs more than what critics found. Also correct agent \
             terms the findings show to be wrong, with the reason. Output JSON only: \
             {{\"lessons\": [{{\"id\": \"short-kebab-id\", \"domain\": \"dialogue\" or null, \
             \"text\": \"…\", \"evidence\": <number of findings and changes behind it>}}], \
             \"terms\": [{{\"term\": \"…\", \"translation\": \"…\", \"reason\": \"…\"}}]}}"
        ),
    }
}

fn kebab(text: &str) -> String {
    let mut id = String::new();
    for character in text.trim().chars() {
        if character.is_ascii_alphanumeric() {
            id.push(character.to_ascii_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    id.trim_end_matches('-').chars().take(48).collect()
}

/// Reads the mentor's reply. Lessons whose identifier the project already
/// has are left out.
#[must_use]
pub fn parse_mentor(reply: &str, existing: &[Lesson], job_id: &str) -> MentorReply {
    #[derive(Deserialize)]
    struct Reply {
        #[serde(default)]
        lessons: Vec<LessonItem>,
        #[serde(default)]
        terms: Vec<TermItem>,
    }
    #[derive(Deserialize)]
    struct LessonItem {
        #[serde(default)]
        id: String,
        #[serde(default)]
        domain: Option<String>,
        #[serde(default)]
        text: String,
        #[serde(default)]
        evidence: Option<u64>,
    }
    #[derive(Deserialize)]
    struct TermItem {
        #[serde(default)]
        term: String,
        #[serde(default)]
        translation: String,
        #[serde(default)]
        reason: String,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        return MentorReply::default();
    };
    let Ok(parsed) = serde_json::from_str::<Reply>(&reply[start..=end]) else {
        return MentorReply::default();
    };
    let mut lessons: Vec<Lesson> = Vec::new();
    for item in parsed.lessons.into_iter().take(MAX_NEW_LESSONS) {
        let id = kebab(if item.id.trim().is_empty() {
            &item.text
        } else {
            &item.id
        });
        let text = item.text.trim().to_owned();
        if id.is_empty()
            || text.is_empty()
            || existing
                .iter()
                .chain(lessons.iter())
                .any(|known| known.id == id)
        {
            continue;
        }
        let mut meta = BTreeMap::new();
        meta.insert("source".to_owned(), "mentor".to_owned());
        meta.insert("job".to_owned(), job_id.to_owned());
        if let Some(evidence) = item.evidence {
            meta.insert("findings".to_owned(), evidence.to_string());
        }
        lessons.push(Lesson {
            id,
            status: LessonStatus::Trial,
            domain: item.domain.as_deref().and_then(Domain::parse),
            text,
            meta,
        });
    }
    let terms = parsed
        .terms
        .into_iter()
        .filter(|item| !item.term.trim().is_empty() && !item.translation.trim().is_empty())
        .map(|item| GlossaryEntry {
            term: item.term.trim().to_owned(),
            translation: item.translation.trim().to_owned(),
            note: Some(if item.reason.trim().is_empty() {
                "mentor".to_owned()
            } else {
                format!("{}; mentor", item.reason.trim())
            }),
            forbidden: Vec::new(),
        })
        .collect();
    MentorReply { lessons, terms }
}

/// Who won a comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Winner {
    /// The version with the lessons.
    Candidate,
    /// The version the job wrote.
    Baseline,
    Tie,
}

fn judge_request(
    unit: &UnitOfWork,
    first: &BTreeMap<usize, String>,
    second: &BTreeMap<usize, String>,
) -> Request {
    let mut body = String::new();
    for (index, line) in unit.lines.iter().enumerate() {
        let (Some(a), Some(b)) = (first.get(&index), second.get(&index)) else {
            continue;
        };
        let _ = writeln!(
            body,
            "L{} {}\n  {}: {}",
            index + 1,
            line.address,
            unit.source_language,
            line.source
        );
        for (code, text) in &line.evidence {
            let _ = writeln!(body, "  {code}: {text}");
        }
        let _ = writeln!(body, "  A: {a}\n  B: {b}");
    }
    Request {
        role: Role::Judge,
        part: None,
        system: format!(
            "You are an experienced {} localization editor judging two versions of the same game \
             text. The Japanese is the original; French and German show the professional \
             decisions on address and gender, and a version that follows them is right.",
            unit.target_language
        ),
        user: format!(
            "{body}\nCompare A and B as a whole: which reads as if it had been written in {} for \
             this game, keeps the meaning of the source, keeps each character's voice, stays \
             consistent, and handles the player character's gender correctly? Output JSON only: \
             {{\"winner\": \"A\" or \"B\" or \"tie\", \"reasons\": \"<under 60 words>\"}}",
            unit.target_language
        ),
    }
}

fn parse_winner(reply: &str) -> Option<char> {
    #[derive(Deserialize)]
    struct Reply {
        winner: String,
    }
    let (start, end) = (reply.find('{')?, reply.rfind('}')?);
    let parsed: Reply = serde_json::from_str(&reply[start..=end]).ok()?;
    match parsed.winner.trim() {
        "A" | "a" => Some('A'),
        "B" | "b" => Some('B'),
        _ => None,
    }
}

/// Localizes a unit again with the lessons its knowledge carries, without
/// writing, and judges it against `baseline` (the job's translations by
/// script line) in both orders. Returns the winner and the tokens used.
///
/// # Errors
///
/// Returns the first provider failure.
pub async fn evaluate_unit(
    caller: &dyn Caller,
    unit: UnitOfWork,
    baseline: &BTreeMap<usize, String>,
) -> Result<(Winner, Usage), ProviderError> {
    let task_lines: BTreeMap<usize, usize> = unit
        .lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.task.map(|task| (task, index)))
        .collect();
    let judged_unit = unit.clone();
    let result = localize(caller, unit, LocalizeOptions::default()).await?;
    let mut usage = result.usage;
    let candidate: BTreeMap<usize, String> = result
        .outcomes
        .into_iter()
        .filter_map(|outcome| Some((*task_lines.get(&outcome.task)?, outcome.target?)))
        .collect();
    let requests = vec![
        judge_request(&judged_unit, &candidate, baseline),
        judge_request(&judged_unit, baseline, &candidate),
    ];
    let replies = caller.call_all(requests).await?;
    let mut score = 0_i32;
    for (order, (reply, spent)) in replies.into_iter().enumerate() {
        usage.add(spent);
        let candidate_letter = if order == 0 { 'A' } else { 'B' };
        match parse_winner(&reply) {
            Some(letter) if letter == candidate_letter => score += 1,
            Some(_) => score -= 1,
            None => {}
        }
    }
    let winner = match score.cmp(&0) {
        std::cmp::Ordering::Greater => Winner::Candidate,
        std::cmp::Ordering::Less => Winner::Baseline,
        std::cmp::Ordering::Equal => Winner::Tie,
    };
    Ok((winner, usage))
}

/// A lesson's status after its evaluation: kept when the units with it won
/// more often than they lost, dropped when they lost more often.
#[must_use]
pub fn verdict(winners: &[Winner]) -> Option<LessonStatus> {
    let wins = winners
        .iter()
        .filter(|winner| **winner == Winner::Candidate)
        .count();
    let losses = winners
        .iter()
        .filter(|winner| **winner == Winner::Baseline)
        .count();
    match wins.cmp(&losses) {
        std::cmp::Ordering::Greater => Some(LessonStatus::Active),
        std::cmp::Ordering::Less => Some(LessonStatus::Dropped),
        std::cmp::Ordering::Equal => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mentor_reply_becomes_trial_lessons_and_term_corrections() {
        let existing = vec![Lesson {
            id: "no-hm".to_owned(),
            status: LessonStatus::Dropped,
            domain: None,
            text: "x".to_owned(),
            meta: BTreeMap::new(),
        }];
        let reply = r#"Here: {"lessons": [
            {"id": "No Hm!", "domain": "dialogue", "text": "Never start with Хм.", "evidence": 7},
            {"id": "", "domain": null, "text": "Oaths of Eorzea stay oaths.", "evidence": 3},
            {"id": "empty", "text": ""}],
          "terms": [{"term": "Crystal Braves", "translation": "Кристальные храбрецы", "reason": "the old one was ungrammatical"},
                    {"term": "", "translation": "x"}]}"#;
        let parsed = parse_mentor(reply, &existing, "job-1");
        assert_eq!(
            parsed.lessons.len(),
            1,
            "no-hm exists already, empty is skipped"
        );
        assert_eq!(parsed.lessons[0].id, "oaths-of-eorzea-stay-oaths");
        assert_eq!(parsed.lessons[0].status, LessonStatus::Trial);
        assert_eq!(
            parsed.lessons[0].meta.get("findings").map(String::as_str),
            Some("3")
        );
        assert_eq!(parsed.terms.len(), 1);
        assert_eq!(
            parsed.terms[0].note.as_deref(),
            Some("the old one was ungrammatical; mentor")
        );
    }

    #[test]
    fn lessons_are_kept_when_they_win_and_dropped_when_they_lose() {
        use Winner::{Baseline, Candidate, Tie};
        assert_eq!(verdict(&[Candidate, Tie]), Some(LessonStatus::Active));
        assert_eq!(
            verdict(&[Candidate, Baseline, Baseline]),
            Some(LessonStatus::Dropped)
        );
        assert_eq!(verdict(&[Candidate, Baseline]), None);
        assert_eq!(verdict(&[]), None);
    }

    #[test]
    fn a_judge_reply_names_its_winner() {
        assert_eq!(
            parse_winner(r#"{"winner": "B", "reasons": "x"}"#),
            Some('B')
        );
        assert_eq!(parse_winner(r#"{"winner": "tie"}"#), None);
        assert_eq!(parse_winner("no json"), None);
    }
}
