//! Study: agents that build the project knowledge from the game and from
//! the project's translations.
//!
//! - **Style**: one researcher per text domain reads samples of the domain in
//!   every client language and writes the domain's style entry.
//! - **Characters**: one researcher per speaker label reads the speaker's
//!   lines sampled across the game and writes a profile: gender, voice from
//!   the Japanese, address from the French and German, and how the character
//!   sounds in the target language.
//! - **Terms**: before a unit is localized, a researcher lists the unit's
//!   terminology the knowledge does not have, reads how the project already
//!   translates each term, decides renderings, and a second request checks
//!   them for grammar before they are written.
//!
//! Researchers read evidence and write knowledge through a
//! [`KnowledgeHost`]; they send requests through the localizer's
//! [`Caller`]. Knowledge written here is agent knowledge: human entries are
//! never changed (see [`crate::knowledge`]).

use std::fmt::Write as _;

use serde::Deserialize;

use crate::client::ProviderError;
use crate::guidance::GlossaryEntry;
use crate::knowledge::{Domain, Knowledge, KnowledgeFile, Section};
use crate::localizer::{Caller, Request, Role, UnitOfWork};

/// Samples a style researcher reads for one domain, at most.
pub const STYLE_SAMPLES: usize = 40;
/// Lines a character researcher reads for one speaker, at most.
pub const CHARACTER_SAMPLES: usize = 30;
/// Terms one unit's study decides, at most.
const MAX_TERMS_PER_UNIT: usize = 40;
/// Existing translations shown for each term.
const CONCORDANCE_LINES: usize = 6;

const EVIDENCE: &str = "\
FINAL FANTASY XIV is written in Japanese; the English, German, and French texts are \
professional localizations. The Japanese shows intent and how characters really speak \
(first-person pronoun, sentence endings, politeness); French and German show decisions the \
English hides (tu/vous and du/Sie, gender agreement, register). A device of one localization, \
such as an English written accent, is that localization's choice, not the character's.";

/// A line read as evidence: its label, its source, and the other client
/// languages.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Sample {
    pub label: String,
    pub source: String,
    pub evidence: Vec<(String, String)>,
}

fn samples_text(samples: &[Sample]) -> String {
    let mut text = String::new();
    for sample in samples {
        if !sample.label.is_empty() {
            let _ = writeln!(text, "{}", sample.label);
        }
        let _ = writeln!(text, "  source: {}", sample.source);
        for (code, line) in &sample.evidence {
            let _ = writeln!(text, "  {code}: {line}");
        }
    }
    text
}

/// Knowledge access for researchers, implemented by the desktop.
pub trait KnowledgeHost: Send + Sync {
    /// The project knowledge as it is now.
    fn knowledge(&self) -> Knowledge;

    /// Translated strings whose source contains `term`, as `(source,
    /// translation)` pairs; empty when the project cannot be searched.
    fn concordance(&self, term: &str, limit: usize) -> Vec<(String, String)>;

    /// Adds agent terms and returns the terms written.
    ///
    /// # Errors
    /// Returns a description when the knowledge cannot be written.
    fn set_terms(&self, entries: &[GlossaryEntry]) -> Result<Vec<String>, String>;

    /// Sets agent character profiles and returns how many were written.
    ///
    /// # Errors
    /// Returns a description when the knowledge cannot be written.
    fn set_characters(&self, profiles: &[(Vec<String>, String)]) -> Result<usize, String>;

    /// Sets one section of an agent file.
    ///
    /// # Errors
    /// Returns a description when the knowledge cannot be written.
    fn set_section(&self, file: KnowledgeFile, section: Section) -> Result<(), String>;
}

fn research_system(target: &str) -> String {
    format!(
        "You are a researcher for a localization of FINAL FANTASY XIV into {target}. You write \
         the project's knowledge from evidence, concisely, as rules with short examples in \
         {target}. {EVIDENCE}"
    )
}

/// A request for the style of one domain from its samples.
#[must_use]
pub fn style_request(domain: Domain, target: &str, knowledge: &str, samples: &[Sample]) -> Request {
    Request {
        role: Role::Research,
        part: None,
        system: research_system(target),
        user: format!(
            "Write the project's style entry for {target} {} ({}). Decide from the samples below \
             and say which evidence decided each rule. Cover what applies: grammatical person and \
             address of the player character, tense, register and tone, sentence shape and \
             length, punctuation and typography of {target}, capitalization, and how to avoid \
             words that agree with the player character's gender. Stay consistent with the \
             project knowledge. Under 300 words; no heading.\n\nProject knowledge:\n{knowledge}\n\n\
             Samples:\n{}",
            domain.as_str(),
            domain.describe(),
            samples_text(samples)
        ),
    }
}

/// A request for one character's profile from their lines.
#[must_use]
pub fn character_request(
    speaker: &str,
    total: usize,
    target: &str,
    knowledge: &str,
    samples: &[Sample],
) -> Request {
    Request {
        role: Role::Research,
        part: None,
        system: research_system(target),
        user: format!(
            "Speaker label {speaker}: {total} lines in the game; {} sampled evenly across them \
             below. Write this character's profile for {target} translators, under 250 words, as \
             plain text without a heading:\n\
             - Who this is, if the lines tell, and gender, with the evidence.\n\
             - Voice: how the character speaks in the Japanese (first-person pronoun, endings, \
             politeness, dialect) and where the English invents a device the Japanese lacks.\n\
             - Address of the player character and of others, from French tu/vous and German \
             du/Sie, and whether it changes.\n\
             - How the character sounds in {target}: register, vocabulary, syntax; what to avoid.\n\
             - Two or three example lines in {target}.\n\
             Stay consistent with the project knowledge.\n\nProject knowledge:\n{knowledge}\n\n\
             Lines:\n{}",
            samples.len(),
            samples_text(samples)
        ),
    }
}

fn unit_listing(unit: &UnitOfWork) -> String {
    let mut text = String::new();
    for line in &unit.lines {
        let _ = writeln!(text, "- {}", line.source);
        for (code, evidence) in &line.evidence {
            if matches!(code.as_str(), "ja" | "fr") {
                let _ = writeln!(text, "  {code}: {evidence}");
            }
        }
    }
    text
}

fn candidates_request(unit: &UnitOfWork) -> Request {
    Request {
        role: Role::Terms,
        part: None,
        system: research_system(&unit.target_language),
        user: format!(
            "{}\n{}\nList the terminology in these lines that must be translated the same way \
             everywhere in the game: names of people, places, organizations, monsters, and items, \
             game-specific mechanics and crafting terms, interface names, and fixed in-world \
             expressions such as oaths. Leave out ordinary words and phrases even when they recur: \
             verbs such as deliver or speak, common nouns, numbers and levels, and instructions. \
             Write each term as it appears in the {} lines, without an article. Output JSON only: \
             {{\"terms\": [\"term\", …]}}",
            unit.title,
            unit_listing(unit),
            unit.source_language
        ),
    }
}

fn decide_request(unit: &UnitOfWork, knowledge: &str, blocks: &str) -> Request {
    let target = &unit.target_language;
    Request {
        role: Role::Terms,
        part: None,
        system: research_system(target),
        user: format!(
            "For each term below, decide its {target} rendering for the whole game. The lines \
             under each term show how the project already translates it; they are machine drafts \
             that may be wrong. Keep a rendering that is correct and already common, so the \
             project stays consistent, and replace one that is wrong (for example, a fire shard \
             and a fire crystal are different items). Use the unit below and the other languages \
             to understand what each term is, and stay consistent with the project knowledge.\n\n\
             Output for each term exactly two lines:\n## <term as given>\n<rendering> | <kind and \
             grammatical note> | <wrong renderings to avoid, separated by ;, never the rendering \
             itself; leave the field empty when there are none>\n\n\
             Project knowledge:\n{knowledge}\n\nUnit:\n{}\nTerms:\n{blocks}",
            unit_listing(unit)
        ),
    }
}

fn check_request(target: &str, decided: &str) -> Request {
    Request {
        role: Role::Terms,
        part: None,
        system: format!(
            "You are a {target} editor checking terminology decisions of a game localization."
        ),
        user: format!(
            "Check each rendering below. It must be grammatical, natural {target} that a player \
             accepts as a name or term in a fantasy game, match what the term is, and suit a \
             person where the term is a name. For example, an organization's name needs a proper \
             noun phrase whose words agree. Fix the entries that fail and keep the others. Output \
             the complete list in the same format and nothing else.\n\n{decided}"
        ),
    }
}

/// Words models write for an empty list of forbidden variants.
const PLACEHOLDERS: [&str; 12] = [
    "none",
    "empty",
    "n/a",
    "-",
    "—",
    "–",
    "нет",
    "пусто",
    "пустое",
    "не указано",
    "отсутствует",
    "—",
];

/// Reads `## term` / `rendering | note | forbidden` entries.
#[must_use]
pub fn parse_decisions(reply: &str) -> Vec<GlossaryEntry> {
    let mut entries = Vec::new();
    let mut term: Option<String> = None;
    for line in reply.lines() {
        let line = line.trim();
        if let Some(heading) = line.strip_prefix("## ") {
            term = Some(heading.trim().to_owned());
            continue;
        }
        let Some(current) = term.take() else {
            continue;
        };
        if line.is_empty() {
            term = Some(current);
            continue;
        }
        let mut fields = line.split('|').map(str::trim);
        let translation = fields.next().unwrap_or_default().to_owned();
        let note = fields
            .next()
            .filter(|note| !note.is_empty())
            .map(str::to_owned);
        // A forbidden variant that is the rendering itself, or a word for
        // "none", would forbid the term's own translation.
        let forbidden = fields
            .next()
            .map(|list| {
                list.split(';')
                    .map(|item| item.trim().trim_matches(['«', '»', '"', '\'']).trim())
                    .filter(|item| {
                        !item.is_empty()
                            && !item.to_lowercase().eq(&translation.to_lowercase())
                            && !PLACEHOLDERS.contains(&item.to_lowercase().as_str())
                    })
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if !current.is_empty() && !translation.is_empty() {
            entries.push(GlossaryEntry {
                term: current,
                translation,
                note,
                forbidden,
            });
        }
    }
    entries
}

fn parse_candidates(reply: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Reply {
        #[serde(default)]
        terms: Vec<String>,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        return Vec::new();
    };
    serde_json::from_str::<Reply>(&reply[start..=end])
        .map(|reply| reply.terms)
        .unwrap_or_default()
}

/// Decides the terms of a unit the knowledge does not have yet and writes
/// them. Returns the terms written.
///
/// # Errors
///
/// Returns the first provider failure.
pub async fn study_terms(
    caller: &dyn Caller,
    host: &dyn KnowledgeHost,
    unit: &UnitOfWork,
) -> Result<Vec<String>, ProviderError> {
    let replies = caller.call_all(vec![candidates_request(unit)]).await?;
    let knowledge = host.knowledge();
    let mut terms: Vec<String> = Vec::new();
    for term in parse_candidates(&replies[0].0) {
        let term = term.trim().to_owned();
        if term.is_empty()
            || term.chars().count() > 60
            || term.chars().any(|c| c.is_ascii_digit())
            || knowledge.has_term(&term)
            || terms.iter().any(|known| known.eq_ignore_ascii_case(&term))
        {
            continue;
        }
        terms.push(term);
        if terms.len() >= MAX_TERMS_PER_UNIT {
            break;
        }
    }
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut blocks = String::new();
    for term in &terms {
        let uses = host.concordance(term, CONCORDANCE_LINES);
        let _ = writeln!(blocks, "### {term}");
        if uses.is_empty() {
            blocks.push_str("  (not translated elsewhere in the project yet)\n");
        }
        for (source, target) in uses {
            let _ = writeln!(blocks, "  source: {source}\n  translation: {target}");
        }
    }
    let knowledge_text = unit.knowledge.as_str();
    let decided = caller
        .call_all(vec![decide_request(unit, knowledge_text, &blocks)])
        .await?;
    let checked = caller
        .call_all(vec![check_request(&unit.target_language, &decided[0].0)])
        .await?;
    let mut entries = parse_decisions(&checked[0].0);
    if entries.is_empty() {
        entries = parse_decisions(&decided[0].0);
    }
    // Only the terms asked about are written.
    entries.retain(|entry| {
        terms
            .iter()
            .any(|term| term.eq_ignore_ascii_case(&entry.term))
    });
    for entry in &mut entries {
        let note = entry.note.take().unwrap_or_default();
        entry.note = Some(if note.is_empty() {
            "study".to_owned()
        } else {
            format!("{note}; study")
        });
    }
    Ok(host.set_terms(&entries).unwrap_or_default())
}

/// Writes one domain's style from its samples.
///
/// # Errors
///
/// Returns the first provider failure.
pub async fn study_style(
    caller: &dyn Caller,
    host: &dyn KnowledgeHost,
    domain: Domain,
    target: &str,
    knowledge: &str,
    samples: &[Sample],
) -> Result<bool, ProviderError> {
    if samples.is_empty() {
        return Ok(false);
    }
    let replies = caller
        .call_all(vec![style_request(domain, target, knowledge, samples)])
        .await?;
    let text = replies[0].0.trim();
    if text.is_empty() {
        return Ok(false);
    }
    Ok(host
        .set_section(
            KnowledgeFile::Style,
            Section::new(domain.as_str(), text).with("source", "study"),
        )
        .is_ok())
}

/// Writes character profiles from their speakers' samples, in parallel.
/// Returns how many were written.
///
/// # Errors
///
/// Returns the first provider failure.
pub async fn study_characters(
    caller: &dyn Caller,
    host: &dyn KnowledgeHost,
    target: &str,
    knowledge: &str,
    speakers: &[(String, usize, Vec<Sample>)],
) -> Result<usize, ProviderError> {
    let requests: Vec<Request> = speakers
        .iter()
        .map(|(speaker, total, samples)| {
            character_request(speaker, *total, target, knowledge, samples)
        })
        .collect();
    if requests.is_empty() {
        return Ok(0);
    }
    let replies = caller.call_all(requests).await?;
    let profiles: Vec<(Vec<String>, String)> = speakers
        .iter()
        .zip(replies)
        .filter(|(_, (reply, _))| !reply.trim().is_empty())
        .map(|((speaker, _, _), (reply, _))| {
            (
                vec![speaker.clone()],
                format!("<!-- aeria: source=study -->\n{}", reply.trim()),
            )
        })
        .collect();
    Ok(host.set_characters(&profiles).unwrap_or_default())
}

/// The story part of a contract, between `<story>` tags, if it has one.
#[must_use]
pub fn contract_story(contract: &str) -> Option<String> {
    let start = contract.find("<story>")? + "<story>".len();
    let end = contract[start..].find("</story>")? + start;
    let story = contract[start..end].trim();
    (!story.is_empty()).then(|| story.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::chat::Usage;
    use crate::localizer::{LineKind, Replies, ScriptLine};

    #[test]
    fn decisions_read_term_rendering_note_and_forbidden_variants() {
        let entries = parse_decisions(
            "Here:\n## Fire Shard\nогненный осколок | предмет, м. р. | огненный кристалл; none; «огненный осколок»\n\n## Crystal Braves\n\nКристальные храбрецы |  | пусто\n## Empty\n",
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].translation, "огненный осколок");
        assert_eq!(entries[0].note.as_deref(), Some("предмет, м. р."));
        assert_eq!(entries[0].forbidden, vec!["огненный кристалл"]);
        assert_eq!(entries[1].term, "Crystal Braves");
        assert_eq!(entries[1].note, None);
        assert!(
            entries[1].forbidden.is_empty(),
            "a placeholder is no variant"
        );
    }

    #[test]
    fn a_contract_keeps_its_story_between_tags() {
        assert_eq!(
            contract_story("x <story> Lyngsath tests the player. </story> y").as_deref(),
            Some("Lyngsath tests the player.")
        );
        assert_eq!(contract_story("no story"), None);
    }

    struct Host {
        written: Mutex<Vec<GlossaryEntry>>,
    }

    impl KnowledgeHost for Host {
        fn knowledge(&self) -> Knowledge {
            Knowledge::from_parts(
                crate::guidance::ProjectGuide::from_files(
                    Ok(None),
                    Ok(Some("term,translation\nAether,Эфир\n".to_owned())),
                ),
                &crate::knowledge::AgentTexts::default(),
            )
        }
        fn concordance(&self, term: &str, _: usize) -> Vec<(String, String)> {
            if term == "Fire Shard" {
                vec![(
                    "Bring a fire shard.".to_owned(),
                    "Принеси огненный кристалл.".to_owned(),
                )]
            } else {
                Vec::new()
            }
        }
        fn set_terms(&self, entries: &[GlossaryEntry]) -> Result<Vec<String>, String> {
            self.written.lock().unwrap().extend(entries.iter().cloned());
            Ok(entries.iter().map(|entry| entry.term.clone()).collect())
        }
        fn set_characters(&self, _: &[(Vec<String>, String)]) -> Result<usize, String> {
            Ok(0)
        }
        fn set_section(&self, _: KnowledgeFile, _: Section) -> Result<(), String> {
            Ok(())
        }
    }

    struct Answers {
        asked: Mutex<Vec<String>>,
    }

    impl Caller for Answers {
        fn call_all(&self, requests: Vec<Request>) -> Replies<'_> {
            Box::pin(async move {
                let mut replies = Vec::new();
                for request in requests {
                    self.asked.lock().unwrap().push(request.user.clone());
                    let reply = if request.user.contains("Output JSON only") {
                        r#"{"terms": ["Fire Shard", "aether", "Lyngsath", "fire shard"]}"#
                    } else if request.user.contains("Check each rendering") {
                        "## Fire Shard\nогненный осколок | предмет | огненный кристалл\n## Lyngsath\nЛингсат | имя |\n## Stranger\nчужой | |"
                    } else {
                        "## Fire Shard\nогненный кристалл | предмет |\n## Lyngsath\nЛингсат | имя |"
                    };
                    replies.push((reply.to_owned(), Usage::default()));
                }
                Ok(replies)
            })
        }
    }

    #[test]
    fn term_study_skips_known_terms_and_writes_the_checked_decisions() {
        let unit = UnitOfWork {
            title: "Quest".to_owned(),
            sheet: "quest/000/Test".to_owned(),
            source_language: "en".to_owned(),
            target_language: "ru".to_owned(),
            lines: vec![ScriptLine {
                kind: LineKind::Speech("LYNGSATH".to_owned()),
                address: "q:1:0:0".to_owned(),
                source: "Bring a fire shard and some aether.".to_owned(),
                evidence: Vec::new(),
                legends: Vec::new(),
                context: Vec::new(),
                current: None,
                note: None,
                memory: Vec::new(),
                task: Some(0),
            }],
            domains: vec![Domain::Dialogue],
            knowledge: String::new(),
            instructions: String::new(),
        };
        let host = Host {
            written: Mutex::new(Vec::new()),
        };
        let caller = Answers {
            asked: Mutex::new(Vec::new()),
        };
        let written = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(study_terms(&caller, &host, &unit))
            .unwrap();
        assert_eq!(written, vec!["Fire Shard", "Lyngsath"]);
        let entries = host.written.lock().unwrap().clone();
        assert_eq!(entries[0].translation, "огненный осколок");
        assert_eq!(entries[0].note.as_deref(), Some("предмет; study"));
        let asked = caller.asked.lock().unwrap().clone();
        assert!(asked[1].contains("Принеси огненный кристалл."));
        assert!(!asked[1].contains("### aether"));
    }
}
