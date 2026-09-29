//! Study: agents that build the project knowledge from the game and from
//! the project's translations.
//!
//! - **Style**: one researcher per text domain reads samples of the domain in
//!   every client language and writes the domain's style entry.
//! - **Characters**: one researcher per speaker label reads the speaker's
//!   lines sampled across the game in every client language and writes a
//!   voice sheet: who the character is in the Japanese, how each
//!   localization makes them sound, the target-language devices that give
//!   the same portrait, address, and gender.
//! - **Terms**: before a unit is localized, one request lists the unit's
//!   terminology the knowledge does not have and decides and checks each
//!   rendering, reading how the project translated similar strings. Names of people keep the
//!   project's rendering; a name that the localizations each invented anew
//!   (an establishment, a nickname with a meaning) gets a rendering and up
//!   to two alternatives a person can choose instead.
//!
//! Researchers read evidence and write knowledge through a
//! [`KnowledgeHost`]; they send requests through the localizer's
//! [`Caller`]. Knowledge written here is agent knowledge: human entries are
//! never changed (see [`crate::knowledge`]).

use std::fmt::Write as _;

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

const EVIDENCE: &str = "\
FINAL FANTASY XIV is written in Japanese; the English, German, and French texts are three \
finished localizations, and each made its own creative choices: names, jokes, and how \
characters sound. The Japanese shows intent and how characters speak (first-person \
pronoun, sentence endings, politeness); each localization shows how it made that work for \
its players (a written accent, dialect words, oaths, pomp, tics) and decisions the \
original leaves open (tu/vous and du/Sie, gender agreement, register). This project is \
another localization: it makes its own choices from all of them, copies none, and never \
makes a character flatter than the original and the localizations make them.";

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
             below. Write this character's voice sheet for {target} writers, under 300 words, as \
             plain text without a heading:\n\
             - Who this is and how the original conceives them (first-person pronoun, endings, \
             politeness, dialect, temperament), with gender and its evidence.\n\
             - How the localizations play them: what each does to make the voice (quote its \
             markers: accent spelling, dialect words, oaths, pomp, tics, rhythm) and what its \
             players imagine.\n\
             - Voice in {target}: four to six concrete devices that give a {target} player the \
             portrait at the strength the character has (vocabulary such as colloquial, trade \
             jargon, archaic, or bookish words and oaths of the same strength; syntax; particles \
             and interjections; forms of address; tics), each with where it appears and a short \
             example. No misspelled words for an accent; stutters and drawn-out words stay.\n\
             - What flattens or caricatures the character.\n\
             - Address of the player character and of others, from French tu/vous and German \
             du/Sie, and whether it changes.\n\
             - Three of the most marked lines in {target}, as marked as the original.\n\
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
            if matches!(code.as_str(), "ja" | "fr" | "de") {
                let _ = writeln!(text, "  {code}: {evidence}");
            }
        }
        // Translations of similar strings show how the project already
        // renders the names and terms they share.
        for memory in line.memory.iter().take(2) {
            let _ = writeln!(
                text,
                "  project translation of a similar string: {} → {}",
                memory.source, memory.target
            );
        }
    }
    text
}

/// One request that lists the unit's terminology the knowledge lacks and
/// decides and checks each rendering, so a unit waits for one answer.
fn terms_request(unit: &UnitOfWork) -> Request {
    let target = &unit.target_language;
    Request {
        role: Role::Terms,
        part: None,
        system: research_system(target),
        user: format!(
            "{}\n{}\nList the terminology in these lines that must be translated the same way \
             everywhere in the game and that the project knowledge below does not have yet: \
             names of people, places, organizations, monsters, and items, game-specific mechanics \
             and crafting terms, interface names, and fixed in-world expressions such as oaths. \
             Leave out ordinary words and phrases even when they recur: verbs such as deliver or \
             speak, common nouns, numbers and levels, and instructions. Write each term as it \
             appears in the {} lines, without an article.\n\
             Decide each term's {target} rendering for the whole game. Translations of similar \
             strings are the project's earlier work: keep a rendering they use when it is \
             correct, and replace one that is wrong (a fire shard and a fire crystal are \
             different items). Use the other languages to understand what each term is.\n\
             Names: compare the name in every language of the unit. A person's name or a \
             transliterated name keeps the project's rendering or is transliterated. Where the \
             localizations each made up their own name (an establishment, a place or a nickname \
             with a meaning, such as a tavern the Japanese calls the drowned dolphin, the English \
             the Drowning Wench, and the German drowned sorrow), decide a {target} name that \
             works for its players, inspired by all of them, and give two alternatives a person \
             could choose instead.\n\
             Every rendering and alternative must be grammatical, natural {target} that a player \
             accepts in a fantasy game, match what the term is, suit a person where the term is a \
             name, and not be unintentionally funny; an organization's name needs a proper noun \
             phrase whose words agree.\n\n\
             Output for each term exactly two lines, and nothing else:\n## <term as given>\n\
             <rendering> | <kind and grammatical note> | <wrong renderings to avoid, separated by \
             ;, never the rendering itself; leave the field empty when there are none> | <for a \
             made-up name only: two alternatives separated by ;>\n\n\
             Project knowledge:\n{}",
            unit.title,
            unit_listing(unit),
            unit.source_language,
            unit.knowledge
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

/// A made-up name a person can choose another rendering for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameChoice {
    pub term: String,
    pub rendering: String,
    pub alternatives: Vec<String>,
}

/// What a unit's term study wrote.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TermStudy {
    /// Terms written to the knowledge.
    pub written: Vec<String>,
    /// Written names with alternatives for a person to choose from.
    pub choices: Vec<NameChoice>,
}

/// Reads `## term` / `rendering | note | forbidden` entries.
#[must_use]
pub fn parse_decisions(reply: &str) -> Vec<GlossaryEntry> {
    parse_decisions_with_choices(reply).0
}

/// Reads `## term` / `rendering | note | forbidden | alternatives` entries,
/// with the alternatives of made-up names.
fn parse_decisions_with_choices(reply: &str) -> (Vec<GlossaryEntry>, Vec<NameChoice>) {
    let mut choices = Vec::new();
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
        let alternatives: Vec<String> = fields
            .next()
            .map(|list| {
                list.split(';')
                    .map(|item| item.trim().trim_matches(['«', '»', '"', '\'']).trim())
                    .filter(|item| {
                        !item.is_empty()
                            && item.to_lowercase()
                                != translation
                                    .trim_matches(['«', '»', '"', '\''])
                                    .to_lowercase()
                            && !PLACEHOLDERS.contains(&item.to_lowercase().as_str())
                    })
                    .take(2)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if !current.is_empty() && !translation.is_empty() {
            if !alternatives.is_empty() {
                choices.push(NameChoice {
                    term: current.clone(),
                    rendering: translation.clone(),
                    alternatives,
                });
            }
            entries.push(GlossaryEntry {
                term: current,
                translation,
                note,
                forbidden,
            });
        }
    }
    (entries, choices)
}

/// Decides the terms of a unit the knowledge does not have yet and writes
/// them. Returns the terms written and the made-up names a person may
/// choose another rendering for.
///
/// # Errors
///
/// Returns the first provider failure.
pub async fn study_terms(
    caller: &dyn Caller,
    host: &dyn KnowledgeHost,
    unit: &UnitOfWork,
) -> Result<TermStudy, ProviderError> {
    let replies = caller.call_all(vec![terms_request(unit)]).await?;
    let knowledge = host.knowledge();
    let (mut entries, mut choices) = parse_decisions_with_choices(&replies[0].0);
    // Only new, plausible terms are written, each once.
    let mut seen: Vec<String> = Vec::new();
    entries.retain(|entry| {
        let term = entry.term.trim();
        let key = term.to_lowercase();
        let keep = !term.is_empty()
            && term.chars().count() <= 60
            && !term.chars().any(|c| c.is_ascii_digit())
            && !knowledge.has_term(term)
            && !seen.contains(&key)
            && seen.len() < MAX_TERMS_PER_UNIT;
        if keep {
            seen.push(key);
        }
        keep
    });
    choices.retain(|choice| {
        entries
            .iter()
            .any(|entry| entry.term == choice.term && entry.translation == choice.rendering)
    });
    for entry in &mut entries {
        let mut note = entry.note.take().unwrap_or_default();
        if let Some(choice) = choices.iter().find(|choice| choice.term == entry.term) {
            if !note.is_empty() {
                note.push_str("; ");
            }
            let _ = write!(note, "or: {}", choice.alternatives.join(" / "));
        }
        entry.note = Some(if note.is_empty() {
            "study".to_owned()
        } else {
            format!("{note}; study")
        });
    }
    let written = host.set_terms(&entries).unwrap_or_default();
    choices.retain(|choice| written.contains(&choice.term));
    Ok(TermStudy { written, choices })
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
    fn made_up_names_keep_up_to_two_alternatives() {
        let (entries, choices) = parse_decisions_with_choices(
            "## Drowning Wench\n«Утопленная русалка» | таверна | Тонущая девка | Залитое горе; «Утопленная русалка»; none\n## Baderon\nБадерон | имя | |\n",
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].translation, "«Утопленная русалка»");
        assert_eq!(entries[0].forbidden, vec!["Тонущая девка"]);
        assert_eq!(
            choices,
            vec![NameChoice {
                term: "Drowning Wench".to_owned(),
                rendering: "«Утопленная русалка»".to_owned(),
                alternatives: vec!["Залитое горе".to_owned()],
            }]
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
                    let reply = "## Fire Shard\nогненный осколок | предмет | огненный кристалл\n## Aether\nэфир | |\n## Lyngsath\nЛингсат | имя |\n## fire shard\nосколок | |\n## Level 5\nуровень 5 | |";
                    replies.push((reply.to_owned(), Usage::default()));
                }
                Ok(replies)
            })
        }
    }

    #[test]
    fn term_study_writes_new_terms_once_in_one_request() {
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
        assert_eq!(written.written, vec!["Fire Shard", "Lyngsath"]);
        assert!(written.choices.is_empty());
        let entries = host.written.lock().unwrap().clone();
        assert_eq!(entries[0].translation, "огненный осколок");
        assert_eq!(entries[0].note.as_deref(), Some("предмет; study"));
        let asked = caller.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "one request per unit");
        assert!(asked[0].contains("Bring a fire shard"));
    }
}
