//! The localizer: translates one unit of work through a contract, part
//! writers running in parallel, narrow critics, and fixes.
//!
//! A unit of work is a quest or cutscene sheet (its whole scene, with the
//! job's strings marked for translation and the others shown as context) or
//! a chunk of other strings. The steps are:
//!
//! 1. **Contract**: one request reads the whole unit and fixes the decisions
//!    every writer must share: address between characters and toward the
//!    player, genders, names and terms, the form of journal entries,
//!    objectives, and system text.
//! 2. **Write**: the unit's strings are split into parts of about
//!    [`PART_LINES`] and each part is written by its own request, all in
//!    parallel; every writer sees the whole unit and the contract.
//! 3. **Structure**: every line is checked as it would be written; refused
//!    lines go back once or twice for correction.
//! 4. **Critics**: three narrow critics read each part in parallel: a blind
//!    reader who sees only the target text, a fidelity check against the
//!    source line, and a check of the player character's gender and of
//!    address.
//! 5. **Fix**: flagged lines of each part are corrected in parallel; only
//!    flagged lines may change.
//! 6. **Recheck**: the critics read the changed and flagged lines again; a
//!    remaining major flag is fixed once more, and a line whose major flag
//!    was not fixed needs review.
//!
//! This module builds the requests and reads the replies; it never talks to
//! a provider. [`localize`] runs the steps over a [`Caller`] that sends
//! requests, in parallel where it can. Nothing is written here: the caller
//! of [`localize`] writes the [`LineOutcome`]s through the project's
//! compare-and-set assisted write.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;

use serde::Deserialize;

use crate::chat::Usage;
use crate::client::ProviderError;
use crate::knowledge::Domain;
use crate::provider::ReasoningEffort;
use crate::search::MemoryMatch;
use crate::style::{PLAYER_CHARACTER, TRANSLATION_STYLE};
use crate::tools::{ContextCell, ReviewLabel};

/// Strings one part writer translates, about.
pub const PART_LINES: usize = 40;
/// Most strings of one unit of work; a larger quest or cutscene sheet is
/// split into several units.
pub const MAX_UNIT_LINES: usize = 240;
/// Lines shown before a part to the blind reader, for continuity.
const READER_LINES_BEFORE: usize = 4;
/// Rounds of structure corrections after writing or fixing.
const STRUCTURE_ROUNDS: usize = 2;

/// What one request does. Each role has its own instructions and the
/// reasoning effort that served it best.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Role {
    Contract,
    Writer,
    Structure,
    Blind,
    Fidelity,
    Player,
    Consistency,
    Fix,
    /// Study of style and characters.
    Research,
    /// Study of a unit's terms.
    Terms,
}

impl Role {
    /// The role's name in events and activity.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Writer => "writer",
            Self::Structure => "structure",
            Self::Blind => "blind",
            Self::Fidelity => "fidelity",
            Self::Player => "player",
            Self::Consistency => "consistency",
            Self::Fix => "fix",
            Self::Research => "research",
            Self::Terms => "terms",
        }
    }

    /// The effort that served the role best: writing and the check of the
    /// player character need the most, the fidelity check the least.
    const fn wanted_effort(self) -> ReasoningEffort {
        match self {
            Self::Writer | Self::Player => ReasoningEffort::High,
            Self::Contract
            | Self::Blind
            | Self::Consistency
            | Self::Fix
            | Self::Research
            | Self::Terms => ReasoningEffort::Medium,
            Self::Structure | Self::Fidelity => ReasoningEffort::Low,
        }
    }
}

const fn effort_rank(effort: ReasoningEffort) -> i32 {
    match effort {
        ReasoningEffort::Minimal => 0,
        ReasoningEffort::Low => 1,
        ReasoningEffort::Medium => 2,
        ReasoningEffort::High => 3,
        ReasoningEffort::XHigh => 4,
    }
}

/// The effort to request for a role: the accepted effort nearest to the
/// role's, the higher one on a tie, never above `ceiling` when one is set,
/// or none when the model accepts none.
#[must_use]
pub fn effort_for(
    role: Role,
    accepted: &[ReasoningEffort],
    ceiling: Option<ReasoningEffort>,
) -> Option<ReasoningEffort> {
    let wanted = effort_rank(role.wanted_effort());
    let limit = ceiling.map_or(i32::MAX, effort_rank);
    let allowed: Vec<ReasoningEffort> = accepted
        .iter()
        .copied()
        .filter(|effort| effort_rank(*effort) <= limit)
        .collect();
    // A ceiling below every accepted effort leaves the lowest one.
    let candidates = if allowed.is_empty() {
        accepted
            .iter()
            .copied()
            .min_by_key(|effort| effort_rank(*effort))
            .into_iter()
            .collect()
    } else {
        allowed
    };
    candidates.into_iter().min_by_key(|effort| {
        let rank = effort_rank(*effort);
        ((rank - wanted).abs(), -rank)
    })
}

/// What kind of text a line is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineKind {
    Journal,
    Objective,
    /// Speech by a speaker label from the row key.
    Speech(String),
    /// A dialogue row with no known role.
    Other,
    /// A string of a sheet that is not dialogue.
    Text,
}

impl LineKind {
    fn label(&self) -> String {
        match self {
            Self::Journal => "[journal entry]".to_owned(),
            Self::Objective => "[objective]".to_owned(),
            Self::Speech(speaker) => speaker.clone(),
            Self::Other => "[other]".to_owned(),
            Self::Text => "[text]".to_owned(),
        }
    }
}

/// One line of a unit's script.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptLine {
    pub kind: LineKind,
    /// `sheet:row:subrow:column`, for people reading events.
    pub address: String,
    /// The source as macro text.
    pub source: String,
    /// The line in the game's other client languages, as `(code, text)`.
    pub evidence: Vec<(String, String)>,
    /// What each macro of the source does.
    pub legends: Vec<String>,
    /// Other cells of the row, for strings that are not dialogue.
    pub context: Vec<ContextCell>,
    /// The line's current translation.
    pub current: Option<String>,
    pub note: Option<String>,
    /// Similar translated strings, most similar first.
    pub memory: Vec<MemoryMatch>,
    /// The job string this line translates, as an index into the job
    /// chunk; `None` for a line shown only as context.
    pub task: Option<usize>,
}

impl ScriptLine {
    /// Whether the French or German text varies with the player character's
    /// gender where the source does not: evidence that a gendered target
    /// language needs a condition here too.
    #[must_use]
    pub fn gender_marked(&self) -> bool {
        !self.source.contains("$gn4")
            && self
                .evidence
                .iter()
                .any(|(code, text)| matches!(code.as_str(), "fr" | "de") && text.contains("$gn4"))
    }
}

/// Everything the localizer knows about one unit of work.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitOfWork {
    /// What the unit is, such as a quest's name and sheet.
    pub title: String,
    /// The sheet of the unit's strings.
    pub sheet: String,
    pub source_language: String,
    pub target_language: String,
    pub lines: Vec<ScriptLine>,
    /// The kinds of text in the unit.
    pub domains: Vec<Domain>,
    /// The project knowledge that applies to the unit, as prompt text.
    pub knowledge: String,
    /// The job's instructions.
    pub instructions: String,
}

/// One request to a model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub role: Role,
    /// The part the request is about, if any.
    pub part: Option<usize>,
    pub system: String,
    pub user: String,
}

/// A critic's finding on one line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Flag {
    /// Index of the line in the script.
    pub line: usize,
    pub role: Role,
    pub major: bool,
    pub problem: String,
    pub hint: Option<String>,
}

/// How a translated line ends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Finish {
    /// The critics left nothing open: the translation is final.
    Final,
    /// A major problem is still open; a person should decide.
    NeedsReview(String),
    /// No valid translation was produced.
    Rejected(String),
}

/// The result for one job string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineOutcome {
    /// Index of the string in the job chunk.
    pub task: usize,
    pub target: Option<String>,
    pub finish: Finish,
}

/// Replies to a batch of requests, in request order, with their usage.
pub type Replies<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<(String, Usage)>, ProviderError>> + Send + 'a>>;

/// Sends requests, in parallel where it can, and returns the replies in
/// request order with their usage. The first failure ends the unit.
pub trait Caller: Send + Sync {
    fn call_all(&self, requests: Vec<Request>) -> Replies<'_>;

    /// Called when the localizer moves to another step.
    fn step(&self, _step: Step) {}
}

/// The steps of [`localize`], reported to the [`Caller`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// The study of a job's scope, before its first unit.
    Study,
    /// The study of a unit's terms, before its contract.
    Terms,
    Contract,
    Writing,
    Reviewing,
    Fixing,
    Rechecking,
}

/// One unit's localization in progress.
#[derive(Clone, Debug)]
pub struct Localization {
    unit: UnitOfWork,
    /// Script line indices of the job strings, per part, in script order.
    parts: Vec<Vec<usize>>,
    contract: String,
    targets: BTreeMap<usize, String>,
}

const EVIDENCE: &str = "\
FINAL FANTASY XIV is written in Japanese; the English, German, and French texts are \
professional localizations. You get the source language you translate and the others as \
evidence. The Japanese shows intent, tone, and how a character really speaks (first-person \
pronoun, sentence endings, politeness). French and German show decisions the English hides: \
tu/vous and du/Sie between speakers and toward the player character, the gender of \
speakers and of the player character, and register. Use them as evidence, not as rules.
Content comes only from the source line. Never move a word, detail, sentence, or example \
from another language into a line, even when the Japanese says more or something else; the \
other languages decide only tone, voice, address, gender, and how to read an ambiguous \
source line. A line with content its source does not have is a mistranslation.";

const MARKED: &str = "\
Lines marked `varies by player gender` are where the French or German change with the \
player character's gender; the translation needs a condition on $gn4 there or a phrasing \
where nothing agrees with it. Other lines can need one too.";

/// What writers keep in mind for each kind of text, besides the project's
/// own style.
fn domain_notes(domains: &[Domain]) -> String {
    let mut notes = String::new();
    for domain in domains {
        let note = match domain {
            Domain::Names => {
                "Names: one rendering per name across the whole game. Follow the project's \
                 transliteration and the terms; translate a name's meaning only where the project \
                 does (descriptive names of monsters and places usually are translated, personal \
                 names are not)."
            }
            Domain::Items => {
                "Items: names follow the pattern of their series, so the pieces of a set, grades, \
                 and variants read alike; names are short and capitalized as the project does; \
                 descriptions keep the game's tone."
            }
            Domain::Actions => {
                "Actions and statuses: the same mechanic always has the same word; tooltips keep \
                 the game's phrasing for effects, durations, and potency; numbers stay in their \
                 macros."
            }
            Domain::Interface => {
                "Interface: as short as the source or shorter, since the space on screen is \
                 limited; the same label always has the same wording; no flourish."
            }
            Domain::Lore => {
                "Lore: literary text in the voice of its writer, in-world and never modern."
            }
            Domain::General
            | Domain::Journal
            | Domain::Objective
            | Domain::System
            | Domain::Dialogue => continue,
        };
        notes.push_str("- ");
        notes.push_str(note);
        notes.push('\n');
    }
    notes
}

const FLAG_FORMAT: &str = "\
Output JSON only: {\"flags\": [{\"line\": \"L12\", \"severity\": \"major\" or \"minor\", \
\"problem\": \"<one sentence>\", \"hint\": \"<optional short direction>\"}]}. Major: a player \
would notice it. Minor: a better wording exists. Go through every line you are asked to \
check; flag only real problems, and an empty list is the right answer when there are none. \
Do not rewrite lines.";

const LINE_FORMAT: &str = "\
Output exactly one line per string, in the form `L12: text`, each on a single line (line \
breaks of the source stay <br>). Nothing else.";

impl Localization {
    /// Prepares a unit. Its job strings are split into parts of about
    /// [`PART_LINES`], in script order.
    #[must_use]
    pub fn new(unit: UnitOfWork) -> Self {
        let tasks: Vec<usize> = unit
            .lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| line.task.map(|_| index))
            .collect();
        let count = tasks.len().div_ceil(PART_LINES).max(1);
        let size = tasks.len().div_ceil(count).max(1);
        let parts = tasks.chunks(size).map(<[usize]>::to_vec).collect();
        Self {
            unit,
            parts,
            contract: String::new(),
            targets: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn unit(&self) -> &UnitOfWork {
        &self.unit
    }

    #[must_use]
    pub fn parts(&self) -> &[Vec<usize>] {
        &self.parts
    }

    #[must_use]
    pub fn contract(&self) -> &str {
        &self.contract
    }

    /// The target written so far for a script line.
    #[must_use]
    pub fn target(&self, line: usize) -> Option<&str> {
        self.targets.get(&line).map(String::as_str)
    }

    fn id(line: usize) -> String {
        format!("L{}", line + 1)
    }

    /// The system message shared by the contract, writers, and fixes.
    fn localizer_system(&self) -> String {
        let unit = &self.unit;
        let mut system = format!(
            "You are a localizer of FINAL FANTASY XIV from {} into {}. You localize the way a \
             professional game writer would: the text must read as if it had been written in \
             {} for this game.\n\n{EVIDENCE}\n\n{TRANSLATION_STYLE}\n\n{PLAYER_CHARACTER}\n\n{}",
            unit.source_language,
            unit.target_language,
            unit.target_language,
            aeria_se::authoring_reference()
        );
        let notes = domain_notes(&unit.domains);
        if !notes.is_empty() {
            let _ = write!(system, "\n\nKinds of text in this unit:\n{notes}");
        }
        if !unit.instructions.trim().is_empty() {
            let _ = write!(
                system,
                "\n\nJob instructions:\n{}",
                unit.instructions.trim()
            );
        }
        if !unit.knowledge.trim().is_empty() {
            let _ = write!(
                system,
                "\n\nProject knowledge, which takes precedence over the style defaults above:\n{}",
                unit.knowledge.trim()
            );
        }
        system
    }

    /// The whole unit: every line with its role, source, evidence, and
    /// what it needs; job strings are marked `translate`.
    #[must_use]
    pub fn script(&self) -> String {
        let mut script = String::new();
        for (index, line) in self.unit.lines.iter().enumerate() {
            if index > 0 {
                script.push('\n');
            }
            let _ = write!(script, "{} {}", Self::id(index), line.kind.label());
            match (line.task, &line.current) {
                (Some(_), _) => script.push_str(" — translate"),
                (None, Some(current)) => {
                    let _ = write!(script, " — context, translated as: {current}");
                }
                (None, None) => script.push_str(" — context, not translated yet"),
            }
            if line.task.is_some() && line.gender_marked() {
                script.push_str(" — varies by player gender");
            }
            let _ = write!(script, "\n  {}: {}", self.unit.source_language, line.source);
            for (code, text) in &line.evidence {
                let _ = write!(script, "\n  {code}: {text}");
            }
            if line.task.is_some() {
                for legend in &line.legends {
                    let _ = write!(script, "\n  macro: {legend}");
                }
                for cell in &line.context {
                    let _ = write!(
                        script,
                        "\n  row context, column {}: {}",
                        cell.column, cell.source
                    );
                }
                if let Some(current) = &line.current {
                    let _ = write!(script, "\n  current draft, unchecked: {current}");
                }
                if let Some(note) = &line.note {
                    let _ = write!(script, "\n  translator note: {note}");
                }
                for memory in &line.memory {
                    let review = match memory.review_state {
                        ReviewLabel::Reviewed => "reviewed",
                        ReviewLabel::NeedsReview => "needs review",
                        ReviewLabel::Draft => "unchecked draft",
                    };
                    let _ = write!(
                        script,
                        "\n  similar string ({:.0} %, {review}): {} → {}",
                        memory.similarity * 100.0,
                        memory.source,
                        memory.target
                    );
                }
            }
            script.push('\n');
        }
        script
    }

    /// The target text of some lines as a player reads it: written targets
    /// first, then current translations.
    fn target_script(&self, lines: impl IntoIterator<Item = usize>) -> String {
        let mut script = String::new();
        for index in lines {
            let line = &self.unit.lines[index];
            let text = self
                .targets
                .get(&index)
                .map(String::as_str)
                .or(line.current.as_deref())
                .unwrap_or("");
            let _ = writeln!(script, "{} {}: {text}", Self::id(index), line.kind.label());
        }
        script
    }

    fn ids(lines: &[usize]) -> String {
        lines
            .iter()
            .map(|line| Self::id(*line))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Step 1: the contract all writers follow.
    #[must_use]
    pub fn contract_request(&self) -> Request {
        let target = &self.unit.target_language;
        Request {
            role: Role::Contract,
            part: None,
            system: self.localizer_system(),
            user: format!(
                "{} — {} lines.\n\n{}\n\nSeveral writers will translate the lines marked \
                 `translate` in parallel, and they must agree on every decision. Write the \
                 contract they all follow, in English with {target} renderings:\n\
                 1. Story, between <story> and </story>: what happens and what the player \
                 learns, tone per character, jokes, wordplay, and callbacks that span lines \
                 (under 120 words); it is kept for the units that follow.\n\
                 2. Address: one line per speaker and addressee in this unit, including the \
                 player character, as `SPEAKER → ADDRESSEE: <form of address in {target}>`, \
                 decided from the project knowledge and the French and German; note where \
                 the address changes on purpose.\n\
                 3. Genders: each speaker's gender, and each person spoken about whose \
                 gender a {target} word would show.\n\
                 4. Names and terms: every name and term in the unit with its {target} \
                 rendering, from the project knowledge where it has one and from the \
                 lines already translated.\n\
                 5. Journal entries, objectives, and system text: the form each uses.\n\
                 Leave out sections that do not apply.",
                self.unit.title,
                self.unit.lines.len(),
                self.script()
            ),
        }
    }

    /// Records the contract.
    pub fn set_contract(&mut self, contract: &str) {
        contract.trim().clone_into(&mut self.contract);
    }

    /// Step 2: one writer per part.
    #[must_use]
    pub fn write_requests(&self) -> Vec<Request> {
        let system = self.localizer_system();
        let script = self.script();
        self.parts
            .iter()
            .enumerate()
            .map(|(part, lines)| Request {
                role: Role::Writer,
                part: Some(part),
                system: system.clone(),
                user: format!(
                    "{script}\n\nContract for this unit, shared with the writers of the other \
                     parts; follow it exactly:\n{}\n\n{MARKED}\n\nWrite these lines in {}: \
                     {}. Write them as one continuous scene with the lines around them. Before \
                     answering, reread your text as a player would and fix what sounds \
                     translated, stiff, or out of character, and every word that assumes the \
                     player character's gender; do this in your head. {LINE_FORMAT}",
                    self.contract,
                    self.unit.target_language,
                    Self::ids(lines)
                ),
            })
            .collect()
    }

    /// Reads `L12: text` lines of a reply, keeping only the given lines.
    fn parse_lines(reply: &str, allowed: &BTreeSet<usize>) -> BTreeMap<usize, String> {
        let mut lines = BTreeMap::new();
        for text in reply.lines() {
            let text = text.trim().trim_start_matches('`');
            let Some(rest) = text.strip_prefix('L') else {
                continue;
            };
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let Ok(number) = digits.parse::<usize>() else {
                continue;
            };
            let Some(body) = rest[digits.len()..].trim_start().strip_prefix(':') else {
                continue;
            };
            let body = body.trim().trim_end_matches('`').trim();
            if let Some(index) = number.checked_sub(1)
                && allowed.contains(&index)
                && !body.is_empty()
            {
                lines.insert(index, body.to_owned());
            }
        }
        lines
    }

    /// Takes a writer's lines for its part.
    pub fn accept_written(&mut self, part: usize, reply: &str) {
        let allowed: BTreeSet<usize> = self.parts[part].iter().copied().collect();
        self.targets.extend(Self::parse_lines(reply, &allowed));
    }

    /// Why a written line cannot be kept, if it cannot.
    fn line_problems(&self, index: usize) -> Vec<String> {
        let line = &self.unit.lines[index];
        let Some(target) = self.targets.get(&index) else {
            return vec!["the line is missing".to_owned()];
        };
        let mut problems = Vec::new();
        if let Some(label) = leading_label(target, line) {
            problems.push(format!(
                "the line starts with a speaker label or marker ({label}); write only the line's text"
            ));
        }
        if target.contains('→') && !line.source.contains('→') {
            problems.push(
                "the line contains two versions joined by an arrow; write only the final text"
                    .to_owned(),
            );
        }
        if let Some(previous) = index.checked_sub(1)
            && self.targets.get(&previous) == Some(target)
            && self.unit.lines[previous].source != line.source
        {
            problems.push(
                "the line repeats the previous line's translation although their sources differ"
                    .to_owned(),
            );
        }
        if let Err(errors) = aeria_se::check_assisted_structure(&line.source, target) {
            problems.extend(errors.into_iter().map(|error| error.message));
        }
        problems
    }

    /// Step 3: lines that cannot be kept as written, with why.
    #[must_use]
    pub fn problems(&self) -> BTreeMap<usize, Vec<String>> {
        self.parts
            .iter()
            .flatten()
            .filter_map(|index| {
                let problems = self.line_problems(*index);
                (!problems.is_empty()).then_some((*index, problems))
            })
            .collect()
    }

    /// A request to correct refused lines.
    #[must_use]
    pub fn structure_request(&self, problems: &BTreeMap<usize, Vec<String>>) -> Request {
        let mut text = String::new();
        for (index, reasons) in problems {
            let line = &self.unit.lines[*index];
            let _ = writeln!(
                text,
                "{} {}\n  {}: {}\n  now: {}\n  problem: {}",
                Self::id(*index),
                line.kind.label(),
                self.unit.source_language,
                line.source,
                self.targets.get(index).map_or("(missing)", String::as_str),
                reasons.join("; ")
            );
            for legend in &line.legends {
                let _ = writeln!(text, "  macro: {legend}");
            }
        }
        Request {
            role: Role::Structure,
            part: None,
            system: self.localizer_system(),
            user: format!(
                "Contract of this unit:\n{}\n\nAeria refused these lines. Write each of them \
                 again in {}.\n\n{text}\n{LINE_FORMAT}",
                self.contract, self.unit.target_language
            ),
        }
    }

    /// Takes corrected lines.
    pub fn accept_structure(&mut self, problems: &BTreeMap<usize, Vec<String>>, reply: &str) {
        let allowed: BTreeSet<usize> = problems.keys().copied().collect();
        self.targets.extend(Self::parse_lines(reply, &allowed));
    }

    /// Drops every line that still cannot be kept, with why.
    pub fn drop_invalid(&mut self) -> BTreeMap<usize, Vec<String>> {
        let problems = self.problems();
        for index in problems.keys() {
            self.targets.remove(index);
        }
        problems
    }

    /// Step 4: the critics of every part, or only of the given lines.
    #[must_use]
    pub fn critic_requests(&self, only: Option<&BTreeSet<usize>>) -> Vec<Request> {
        let mut requests = Vec::new();
        for (part, lines) in self.parts.iter().enumerate() {
            let checked: Vec<usize> = lines
                .iter()
                .copied()
                .filter(|line| self.targets.contains_key(line))
                .filter(|line| only.is_none_or(|only| only.contains(line)))
                .collect();
            if checked.is_empty() {
                continue;
            }
            requests.push(self.blind_request(part, lines, &checked));
            requests.push(self.fidelity_request(part, &checked));
            requests.push(self.player_request(part, &checked));
            if !self.unit.knowledge.trim().is_empty() {
                requests.push(self.consistency_request(part, &checked));
            }
        }
        requests
    }

    fn blind_request(&self, part: usize, lines: &[usize], checked: &[usize]) -> Request {
        let first = lines[0];
        let shown = first.saturating_sub(READER_LINES_BEFORE)..=lines[lines.len() - 1];
        let target = &self.unit.target_language;
        Request {
            role: Role::Blind,
            part: Some(part),
            system: format!(
                "You are a {target}-speaking player and editor reading a game's text. You do \
                 not see any original; you judge the {target} as {target} text."
            ),
            user: format!(
                "{}\n{}\nCheck lines {}; the others are context. Flag lines that read as a \
                 translation (calques, stiff or bookish where it should be spoken, word order \
                 of another language), are unclear, contradict or do not answer the lines \
                 around them, or sound out of character.\n{FLAG_FORMAT}",
                self.voice_note(),
                self.target_script(shown),
                Self::ids(checked)
            ),
        }
    }

    fn voice_note(&self) -> String {
        if self.contract.is_empty() {
            String::new()
        } else {
            format!(
                "What the unit is and how its characters speak:\n{}\n",
                self.contract
            )
        }
    }

    fn fidelity_request(&self, part: usize, checked: &[usize]) -> Request {
        let mut body = String::new();
        for index in checked {
            let line = &self.unit.lines[*index];
            let _ = writeln!(
                body,
                "{} {}\n  {}: {}\n  {}: {}",
                Self::id(*index),
                line.kind.label(),
                self.unit.source_language,
                line.source,
                self.unit.target_language,
                self.targets.get(index).map_or("", String::as_str)
            );
        }
        Request {
            role: Role::Fidelity,
            part: Some(part),
            system: format!(
                "You compare {} game text with its {} source, line by line.",
                self.unit.target_language, self.unit.source_language
            ),
            user: format!(
                "{body}\nFor each line, flag it when the translation says something its own \
                 source line does not say (for example content of another line or another \
                 language version), loses or changes the meaning, drops a joke, oath, hint, or \
                 callback, or misreads who speaks to whom. Free rewording that keeps meaning \
                 and tone is correct.\n{FLAG_FORMAT}"
            ),
        }
    }

    fn player_request(&self, part: usize, checked: &[usize]) -> Request {
        let mut body = String::new();
        for index in checked {
            let line = &self.unit.lines[*index];
            let _ = write!(body, "{} {}", Self::id(*index), line.kind.label());
            if line.gender_marked() {
                body.push_str(" — varies by player gender");
            }
            let _ = write!(body, "\n  {}: {}", self.unit.source_language, line.source);
            for (code, text) in &line.evidence {
                if matches!(code.as_str(), "fr" | "de") {
                    let _ = write!(body, "\n  {code}: {text}");
                }
            }
            let _ = writeln!(
                body,
                "\n  {}: {}",
                self.unit.target_language,
                self.targets.get(index).map_or("", String::as_str)
            );
        }
        Request {
            role: Role::Player,
            part: Some(part),
            system: format!(
                "You check {} game text for the player character's gender and for forms of \
                 address.",
                self.unit.target_language
            ),
            user: format!(
                "{PLAYER_CHARACTER}\n\nProject knowledge:\n{}\n\nContract of this unit \
                 (address, genders, names):\n{}\n\n{body}\n{MARKED} French past tenses with \
                 avoir do not agree, so check every line: every word that refers to or \
                 agrees with the player character (verbs, adjectives, participles, nouns for \
                 a person, pronouns) and every form of address. Flag a line when a word \
                 assumes the player character's gender without a condition on $gn4, when \
                 address breaks the contract or the project knowledge, or when a speaker's \
                 own gender is wrong.\n{FLAG_FORMAT}",
                self.unit.knowledge.trim(),
                self.contract
            ),
        }
    }

    fn consistency_request(&self, part: usize, checked: &[usize]) -> Request {
        let mut body = String::new();
        for index in checked {
            let line = &self.unit.lines[*index];
            let _ = writeln!(
                body,
                "{} {}\n  {}: {}\n  {}: {}",
                Self::id(*index),
                line.kind.label(),
                self.unit.source_language,
                line.source,
                self.unit.target_language,
                self.targets.get(index).map_or("", String::as_str)
            );
        }
        Request {
            role: Role::Consistency,
            part: Some(part),
            system: format!(
                "You check {} game text against the project's decisions: its terms, its \
                 characters' voices, its style, and the lessons it learned.",
                self.unit.target_language
            ),
            user: format!(
                "Project knowledge:\n{}\n\nContract of this unit:\n{}\n\n{body}\nFlag lines that \
                 contradict the project knowledge or the contract: a term or name rendered \
                 otherwise than the terms say or than elsewhere in these lines, a forbidden \
                 variant, a character who does not sound like their profile, a line that breaks \
                 the style of its kind of text, or one that repeats a mistake a lesson describes. \
                 When the knowledge itself seems wrong, such as an ungrammatical term, flag the \
                 line and start the problem with KNOWLEDGE:.\n{FLAG_FORMAT}",
                self.unit.knowledge.trim(),
                self.contract
            ),
        }
    }

    /// Reads a critic's flags on the lines it was asked to check.
    #[must_use]
    pub fn parse_flags(&self, request: &Request, reply: &str) -> Vec<Flag> {
        #[derive(Deserialize)]
        struct Reply {
            #[serde(default)]
            flags: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            line: String,
            #[serde(default)]
            severity: String,
            #[serde(default)]
            problem: String,
            #[serde(default)]
            hint: Option<String>,
        }
        let Some(part) = request.part else {
            return Vec::new();
        };
        let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
            return Vec::new();
        };
        let Ok(parsed) = serde_json::from_str::<Reply>(&reply[start..=end]) else {
            return Vec::new();
        };
        let lines: BTreeSet<usize> = self.parts[part].iter().copied().collect();
        parsed
            .flags
            .into_iter()
            .filter_map(|item| {
                let number: usize = item.line.trim().trim_start_matches('L').parse().ok()?;
                let line = number.checked_sub(1).filter(|line| lines.contains(line))?;
                (!item.problem.trim().is_empty()).then(|| Flag {
                    line,
                    role: request.role,
                    major: item.severity.eq_ignore_ascii_case("major"),
                    problem: item.problem.trim().to_owned(),
                    hint: item.hint.filter(|hint| !hint.trim().is_empty()),
                })
            })
            .collect()
    }

    /// Step 5: one fix per part with flagged lines.
    #[must_use]
    pub fn fix_requests(&self, flags: &[Flag]) -> Vec<Request> {
        let system = self.localizer_system();
        let mut requests = Vec::new();
        for (part, lines) in self.parts.iter().enumerate() {
            let flagged: BTreeSet<usize> = flags
                .iter()
                .filter(|flag| lines.contains(&flag.line))
                .map(|flag| flag.line)
                .collect();
            if flagged.is_empty() {
                continue;
            }
            let mut text = String::new();
            for index in &flagged {
                let line = &self.unit.lines[*index];
                let _ = writeln!(
                    text,
                    "{} {}\n  {}: {}\n  now: {}",
                    Self::id(*index),
                    line.kind.label(),
                    self.unit.source_language,
                    line.source,
                    self.targets.get(index).map_or("", String::as_str)
                );
                for flag in flags.iter().filter(|flag| flag.line == *index) {
                    let _ = write!(text, "  - {}", flag.problem);
                    if let Some(hint) = &flag.hint {
                        let _ = write!(text, " Hint: {hint}");
                    }
                    text.push('\n');
                }
            }
            requests.push(Request {
                role: Role::Fix,
                part: Some(part),
                system: system.clone(),
                user: format!(
                    "Contract of this unit:\n{}\n\nYour text for this part:\n{}\nEditors \
                     flagged these lines:\n{text}\nFix what is right and ignore a flag that is \
                     wrong. Change only the flagged lines and keep each line's content to its \
                     own source. Output only the lines you change, as `L12: text`, one per \
                     line, or NO CHANGES.",
                    self.contract,
                    self.target_script(lines.iter().copied())
                ),
            });
        }
        requests
    }

    /// Takes a fix's changes to flagged lines of its part.
    pub fn accept_fix(&mut self, request: &Request, flags: &[Flag], reply: &str) {
        let Some(part) = request.part else {
            return;
        };
        let allowed: BTreeSet<usize> = flags
            .iter()
            .map(|flag| flag.line)
            .filter(|line| self.parts[part].contains(line))
            .collect();
        self.targets.extend(Self::parse_lines(reply, &allowed));
    }

    /// The written targets, to find out later which lines changed.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<usize, String> {
        self.targets.clone()
    }

    /// Lines whose target differs from a snapshot.
    #[must_use]
    pub fn changed_since(&self, snapshot: &BTreeMap<usize, String>) -> BTreeSet<usize> {
        self.targets
            .iter()
            .filter(|(line, target)| snapshot.get(line) != Some(target))
            .map(|(line, _)| *line)
            .collect()
    }

    /// The result for every job string: final, needing review because of an
    /// open major flag, or rejected with why.
    #[must_use]
    pub fn outcomes(
        &self,
        open: &[Flag],
        refused: &BTreeMap<usize, Vec<String>>,
    ) -> Vec<LineOutcome> {
        self.parts
            .iter()
            .flatten()
            .filter_map(|index| {
                let task = self.unit.lines[*index].task?;
                let target = self.targets.get(index).cloned();
                let finish = if target.is_none() {
                    Finish::Rejected(refused.get(index).map_or_else(
                        || "no valid translation was produced".to_owned(),
                        |reasons| reasons.join("; "),
                    ))
                } else {
                    let problems: Vec<&str> = open
                        .iter()
                        .filter(|flag| flag.line == *index && flag.major)
                        .map(|flag| flag.problem.as_str())
                        .collect();
                    if problems.is_empty() {
                        Finish::Final
                    } else {
                        Finish::NeedsReview(problems.join(" "))
                    }
                };
                Some(LineOutcome {
                    task,
                    target,
                    finish,
                })
            })
            .collect()
    }
}

/// A speaker label or role marker at the start of a written line, such as
/// `ALISAIE:` or `[objective]:`.
fn leading_label(target: &str, line: &ScriptLine) -> Option<String> {
    let trimmed = target.trim_start();
    let head = trimmed.split(':').next()?;
    if head.len() == trimmed.len() || head.len() > 48 {
        return None;
    }
    let bare = head
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();
    if bare.is_empty()
        || !bare
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' ')
    {
        return None;
    }
    let speaker =
        matches!(&line.kind, LineKind::Speech(speaker) if bare.eq_ignore_ascii_case(speaker));
    let upper = bare.len() > 1
        && bare.chars().any(|c| c.is_ascii_alphabetic())
        && !bare.chars().any(|c| c.is_ascii_lowercase());
    let marker = matches!(bare, "journal entry" | "objective" | "other" | "text");
    let is_label = speaker || upper || marker;
    // A label only counts when the source does not start the same way.
    (is_label && !line.source.trim_start().starts_with(head.trim())).then(|| bare.to_owned())
}

/// What [`localize`] returns.
#[derive(Clone, Debug)]
pub struct LocalizeResult {
    pub outcomes: Vec<LineOutcome>,
    /// The unit's contract, whose story is kept for later units.
    pub contract: String,
    pub usage: Usage,
    /// Flags raised in the first review, for the job's record.
    pub flags: Vec<Flag>,
}

async fn call(
    caller: &dyn Caller,
    requests: Vec<Request>,
    usage: &mut Usage,
) -> Result<Vec<String>, ProviderError> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    let replies = caller.call_all(requests).await?;
    Ok(replies
        .into_iter()
        .map(|(reply, spent)| {
            usage.add(spent);
            reply
        })
        .collect())
}

async fn correct_structure(
    caller: &dyn Caller,
    localization: &mut Localization,
    usage: &mut Usage,
) -> Result<BTreeMap<usize, Vec<String>>, ProviderError> {
    for _ in 0..STRUCTURE_ROUNDS {
        let problems = localization.problems();
        if problems.is_empty() {
            break;
        }
        let request = localization.structure_request(&problems);
        let replies = call(caller, vec![request], usage).await?;
        localization.accept_structure(&problems, &replies[0]);
    }
    Ok(localization.drop_invalid())
}

async fn review(
    caller: &dyn Caller,
    localization: &Localization,
    only: Option<&BTreeSet<usize>>,
    usage: &mut Usage,
) -> Result<Vec<Flag>, ProviderError> {
    let requests = localization.critic_requests(only);
    let replies = call(caller, requests.clone(), usage).await?;
    Ok(requests
        .iter()
        .zip(&replies)
        .flat_map(|(request, reply)| localization.parse_flags(request, reply))
        .collect())
}

async fn fix(
    caller: &dyn Caller,
    localization: &mut Localization,
    flags: &[Flag],
    usage: &mut Usage,
) -> Result<(), ProviderError> {
    let requests = localization.fix_requests(flags);
    let replies = call(caller, requests.clone(), usage).await?;
    for (request, reply) in requests.iter().zip(&replies) {
        localization.accept_fix(request, flags, reply);
    }
    Ok(())
}

/// Localizes one unit of work: contract, parallel writers, structure
/// corrections, critics, fixes, and a recheck of what changed.
///
/// # Errors
///
/// Returns the first provider failure; nothing is written in that case.
pub async fn localize(
    caller: &dyn Caller,
    unit: UnitOfWork,
) -> Result<LocalizeResult, ProviderError> {
    let mut usage = Usage::default();
    let mut localization = Localization::new(unit);

    caller.step(Step::Contract);
    let contract = call(caller, vec![localization.contract_request()], &mut usage).await?;
    localization.set_contract(&contract[0]);

    caller.step(Step::Writing);
    let requests = localization.write_requests();
    let replies = call(caller, requests.clone(), &mut usage).await?;
    for (request, reply) in requests.iter().zip(&replies) {
        if let Some(part) = request.part {
            localization.accept_written(part, reply);
        }
    }
    let mut refused = correct_structure(caller, &mut localization, &mut usage).await?;

    caller.step(Step::Reviewing);
    let first = review(caller, &localization, None, &mut usage).await?;
    caller.step(Step::Fixing);
    let before_fix = localization.snapshot();
    fix(caller, &mut localization, &first, &mut usage).await?;
    refused.extend(correct_structure(caller, &mut localization, &mut usage).await?);

    caller.step(Step::Rechecking);
    let mut touched = localization.changed_since(&before_fix);
    touched.extend(first.iter().map(|flag| flag.line));
    let second: Vec<Flag> = review(caller, &localization, Some(&touched), &mut usage)
        .await?
        .into_iter()
        .filter(|flag| flag.major)
        .collect();
    let before_second_fix = localization.snapshot();
    fix(caller, &mut localization, &second, &mut usage).await?;
    refused.extend(correct_structure(caller, &mut localization, &mut usage).await?);
    // A major flag counts as settled when its line changed in the second
    // fix; otherwise a person decides.
    let changed = localization.changed_since(&before_second_fix);
    let open: Vec<Flag> = second
        .into_iter()
        .filter(|flag| !changed.contains(&flag.line))
        .collect();

    Ok(LocalizeResult {
        outcomes: localization.outcomes(&open, &refused),
        contract: localization.contract().to_owned(),
        usage,
        flags: first,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn line(kind: LineKind, source: &str, task: Option<usize>) -> ScriptLine {
        ScriptLine {
            kind,
            address: format!("quest/000/Test:{}:0:1", task.unwrap_or(99)),
            source: source.to_owned(),
            evidence: Vec::new(),
            legends: Vec::new(),
            context: Vec::new(),
            current: None,
            note: None,
            memory: Vec::new(),
            task,
        }
    }

    fn unit(lines: Vec<ScriptLine>) -> UnitOfWork {
        UnitOfWork {
            title: "Quest \"Test\" (quest/000/Test)".to_owned(),
            sheet: "quest/000/Test".to_owned(),
            source_language: "en".to_owned(),
            target_language: "ru".to_owned(),
            lines,
            domains: vec![Domain::Dialogue],
            knowledge: String::new(),
            instructions: String::new(),
        }
    }

    #[test]
    fn efforts_follow_the_role_within_what_the_model_accepts() {
        use ReasoningEffort::{High, Low, Medium, Minimal};
        assert_eq!(
            effort_for(Role::Writer, &[Low, Medium, High], None),
            Some(High)
        );
        assert_eq!(
            effort_for(Role::Fidelity, &[Low, Medium, High], None),
            Some(Low)
        );
        assert_eq!(effort_for(Role::Blind, &[Low, High], None), Some(High));
        assert_eq!(effort_for(Role::Writer, &[Minimal, Low], None), Some(Low));
        assert_eq!(effort_for(Role::Writer, &[], Some(High)), None);
        // The job's effort is a ceiling.
        assert_eq!(
            effort_for(Role::Writer, &[Low, Medium, High], Some(Medium)),
            Some(Medium)
        );
        assert_eq!(
            effort_for(Role::Fidelity, &[Low, Medium, High], Some(Medium)),
            Some(Low)
        );
        assert_eq!(
            effort_for(Role::Writer, &[Medium, High], Some(Low)),
            Some(Medium)
        );
    }

    #[test]
    fn job_strings_split_into_even_parts_and_context_lines_are_not_written() {
        let mut lines = vec![line(LineKind::Journal, "Context.", None)];
        lines.extend((0..90).map(|task| line(LineKind::Speech("A".into()), "Hello.", Some(task))));
        let localization = Localization::new(unit(lines));
        let sizes: Vec<usize> = localization.parts().iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![30, 30, 30]);
        assert!(
            localization
                .parts()
                .iter()
                .flatten()
                .all(|index| *index != 0)
        );
        assert!(
            localization
                .script()
                .contains("L1 [journal entry] — context, not translated yet")
        );
        assert!(localization.script().contains("L2 A — translate"));
    }

    #[test]
    fn writers_keep_only_their_lines_in_the_expected_form() {
        let lines = vec![
            line(LineKind::Speech("A".into()), "One.", Some(0)),
            line(LineKind::Speech("A".into()), "Two.", Some(1)),
        ];
        let mut localization = Localization::new(unit(lines));
        localization.accept_written(
            0,
            "Here you go:\nL1: Раз.\n`L2: Два.`\nL3: Три.\nL9 nothing",
        );
        assert_eq!(localization.target(0), Some("Раз."));
        assert_eq!(localization.target(1), Some("Два."));
        assert_eq!(localization.target(2), None);
    }

    #[test]
    fn labels_arrows_repeats_and_broken_macros_are_refused() {
        let lines = vec![
            line(LineKind::Speech("ALISAIE".into()), "Come.", Some(0)),
            line(LineKind::Speech("ALISAIE".into()), "Go.", Some(1)),
            line(LineKind::Speech("ALISAIE".into()), "Wait.", Some(2)),
            line(
                LineKind::Journal,
                "Hi, <split \" \" 1><string $gs1></split>.",
                Some(3),
            ),
            line(LineKind::Journal, "Fine.", Some(4)),
        ];
        let mut localization = Localization::new(unit(lines));
        localization.accept_written(
            0,
            "L1: ALISAIE: Идём.\nL2: Уходи.\nL3: Уходи.\nL4: Привет.\nL5: Хорошо → Ладно.",
        );
        let problems = localization.problems();
        assert!(problems[&0][0].contains("speaker label"));
        assert!(!problems.contains_key(&1));
        assert!(problems[&2][0].contains("repeats"));
        assert!(!problems[&3].is_empty());
        assert!(problems[&4][0].contains("arrow"));
        let dropped = localization.drop_invalid();
        assert_eq!(dropped.len(), 4);
        assert_eq!(localization.target(1), Some("Уходи."));
    }

    #[test]
    fn a_source_that_starts_with_a_label_is_not_refused() {
        let lines = vec![line(LineKind::Text, "HP: Restores health.", Some(0))];
        let mut localization = Localization::new(unit(lines));
        localization.accept_written(0, "L1: HP: Восстанавливает здоровье.");
        assert!(localization.problems().is_empty());
    }

    #[test]
    fn gender_marks_come_from_french_and_german() {
        let mut marked = line(LineKind::Speech("A".into()), "You're ready.", Some(0));
        marked.evidence = vec![
            ("ja".into(), "準備はいい？".into()),
            ("fr".into(), "Tu es prêt<if $gn4>e</if>.".into()),
        ];
        assert!(marked.gender_marked());
        let mut own = marked.clone();
        own.source = "<if $gn4>She<else>He</if> is ready.".into();
        assert!(!own.gender_marked());
        let localization = Localization::new(unit(vec![marked]));
        assert!(localization.script().contains("— varies by player gender"));
    }

    #[test]
    fn flags_are_read_only_for_the_part_and_fixes_change_only_flagged_lines() {
        let lines = vec![
            line(LineKind::Speech("A".into()), "One.", Some(0)),
            line(LineKind::Speech("A".into()), "Two.", Some(1)),
        ];
        let mut localization = Localization::new(unit(lines));
        localization.accept_written(0, "L1: Раз.\nL2: Два.");
        let request = &localization.critic_requests(None)[0];
        let flags = localization.parse_flags(
            request,
            r#"Sure. {"flags": [{"line": "L2", "severity": "major", "problem": "Wrong."},
                {"line": "L7", "severity": "major", "problem": "Elsewhere."},
                {"line": "L1", "severity": "minor", "problem": ""}]}"#,
        );
        assert_eq!(flags.len(), 1);
        assert_eq!((flags[0].line, flags[0].major), (1, true));
        let fixes = localization.fix_requests(&flags);
        assert_eq!(fixes.len(), 1);
        localization.accept_fix(&fixes[0], &flags, "L1: Один.\nL2: Второй.");
        assert_eq!(localization.target(0), Some("Раз."));
        assert_eq!(localization.target(1), Some("Второй."));
    }

    /// Answers each request by role and records what it was asked.
    struct Script {
        asked: Mutex<Vec<Role>>,
        steps: Mutex<Vec<Step>>,
        player_flag: bool,
    }

    impl Caller for Script {
        fn call_all(&self, requests: Vec<Request>) -> Replies<'_> {
            Box::pin(async move {
                let mut replies = Vec::new();
                for request in requests {
                    self.asked.lock().unwrap().push(request.role);
                    let reply = match request.role {
                        Role::Contract => "Address: A → player: ты".to_owned(),
                        Role::Writer => "L1: Ты пришёл.\nL2: Хорошо.".to_owned(),
                        Role::Player if self.player_flag => {
                            r#"{"flags": [{"line": "L1", "severity": "major", "problem": "Assumes a male player."}]}"#.to_owned()
                        }
                        Role::Fix if request.user.contains("Ты пришёл") => "NO CHANGES".to_owned(),
                        _ => r#"{"flags": []}"#.to_owned(),
                    };
                    replies.push((
                        reply,
                        Usage {
                            prompt_tokens: 10,
                            completion_tokens: 1,
                        },
                    ));
                }
                Ok(replies)
            })
        }

        fn step(&self, step: Step) {
            self.steps.lock().unwrap().push(step);
        }
    }

    fn run(caller: &Script) -> LocalizeResult {
        let lines = vec![
            line(LineKind::Speech("A".into()), "You came.", Some(0)),
            line(LineKind::Speech("A".into()), "Good.", Some(1)),
        ];
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(localize(caller, unit(lines)))
            .unwrap()
    }

    #[test]
    fn a_clean_unit_is_final_after_one_review() {
        let caller = Script {
            asked: Mutex::new(Vec::new()),
            steps: Mutex::new(Vec::new()),
            player_flag: false,
        };
        let result = run(&caller);
        assert!(
            result
                .outcomes
                .iter()
                .all(|outcome| outcome.finish == Finish::Final)
        );
        assert_eq!(result.outcomes[0].target.as_deref(), Some("Ты пришёл."));
        let asked = caller.asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![
                Role::Contract,
                Role::Writer,
                Role::Blind,
                Role::Fidelity,
                Role::Player
            ]
        );
        assert_eq!(result.usage.prompt_tokens, 50);
        assert_eq!(
            caller.steps.lock().unwrap().clone(),
            vec![
                Step::Contract,
                Step::Writing,
                Step::Reviewing,
                Step::Fixing,
                Step::Rechecking
            ]
        );
    }

    #[test]
    fn a_major_flag_the_fixes_leave_open_needs_review() {
        let caller = Script {
            asked: Mutex::new(Vec::new()),
            steps: Mutex::new(Vec::new()),
            player_flag: true,
        };
        let result = run(&caller);
        assert_eq!(
            result.outcomes[0].finish,
            Finish::NeedsReview("Assumes a male player.".to_owned())
        );
        assert_eq!(result.outcomes[1].finish, Finish::Final);
        assert_eq!(result.flags.len(), 1);
    }
}
