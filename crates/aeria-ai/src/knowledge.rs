//! Project knowledge: what the project decided about style, terms,
//! characters, and story, and the lessons its agents learned.
//!
//! Knowledge has two layers:
//!
//! - **Human**: `aeria-guidance.md`, `aeria-glossary.csv`, and
//!   `aeria-voices.md` at the repository root (see [`crate::guidance`] and
//!   [`crate::voices`]). People write them; agents never change them, so an
//!   entry there is locked.
//! - **Agents**: the files of the `aeria-knowledge` directory, written by
//!   Aeria's agents without a per-change approval: `style.md` (one section
//!   per text domain), `terms.csv` (Glossary Format v1), `characters.md`
//!   (Voice Profiles Format v1), `story.md` (one section per quest or
//!   cutscene sheet), and `lessons.md` (one section per lesson). The format
//!   is described in `docs/formats/knowledge-v1.md`.
//!
//! Where both layers have an entry for the same term or speaker, the human
//! one wins. Agent files are rewritten atomically under a process-wide lock,
//! so parallel job lanes never lose each other's entries.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::guidance::{GlossaryEntry, ProjectGuide, parse_glossary, write_glossary};
use crate::voices::{VoiceProfile, VoiceProfiles, change_voices, parse_voices, speaker_label};

/// The agent layer's directory at the repository root.
pub const KNOWLEDGE_DIR: &str = "aeria-knowledge";
/// Largest agent knowledge file read.
pub const MAX_KNOWLEDGE_BYTES: u64 = 4 * 1024 * 1024;
/// Longest text of one entry placed in a request.
const MAX_ENTRY_PROMPT_CHARS: usize = 2_500;
/// Most glossary entries placed in one request.
const MAX_PROMPT_TERMS: usize = 80;
/// Most character profiles placed in one request.
const MAX_PROMPT_CHARACTERS: usize = 12;
/// Most lessons placed in one request.
const MAX_PROMPT_LESSONS: usize = 20;
/// Most stories of other sheets placed in one request.
const MAX_PROMPT_STORIES: usize = 4;

/// Serializes writes to agent files within the process.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// An agent knowledge file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnowledgeFile {
    Style,
    Terms,
    Characters,
    Story,
    Lessons,
}

impl KnowledgeFile {
    /// The file name inside [`KNOWLEDGE_DIR`].
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Style => "style.md",
            Self::Terms => "terms.csv",
            Self::Characters => "characters.md",
            Self::Story => "story.md",
            Self::Lessons => "lessons.md",
        }
    }

    fn path(self, root: &Path) -> PathBuf {
        root.join(KNOWLEDGE_DIR).join(self.file_name())
    }
}

/// A kind of game text with its own conventions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Domain {
    /// Conventions for all text.
    General,
    /// Quest journal entries.
    Journal,
    /// Quest objectives.
    Objective,
    /// System and tutorial messages in quests.
    System,
    /// Spoken lines of quests and cutscenes.
    Dialogue,
    /// Names of people, places, monsters, and factions.
    Names,
    /// Items, their names and descriptions.
    Items,
    /// Actions, traits, statuses, and other mechanics.
    Actions,
    /// Interface labels, system messages, and logs.
    Interface,
    /// Books, cards, descriptions, and other lore.
    Lore,
}

impl Domain {
    pub const ALL: [Self; 10] = [
        Self::General,
        Self::Journal,
        Self::Objective,
        Self::System,
        Self::Dialogue,
        Self::Names,
        Self::Items,
        Self::Actions,
        Self::Interface,
        Self::Lore,
    ];

    /// The section key in `style.md`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Journal => "journal",
            Self::Objective => "objective",
            Self::System => "system",
            Self::Dialogue => "dialogue",
            Self::Names => "names",
            Self::Items => "items",
            Self::Actions => "actions",
            Self::Interface => "interface",
            Self::Lore => "lore",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|domain| domain.as_str().eq_ignore_ascii_case(text.trim()))
    }

    /// What the domain covers, for researchers and writers.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::General => "conventions for all text: typography, numbers, register",
            Self::Journal => "quest journal entries, which tell the player's story",
            Self::Objective => "quest objectives, the short tasks in the quest log",
            Self::System => "system and tutorial messages shown during quests",
            Self::Dialogue => "spoken lines of quests and cutscenes",
            Self::Names => "names of people, places, monsters, factions, and events",
            Self::Items => "item names and descriptions, whose series must stay consistent",
            Self::Actions => "actions, traits, statuses, and mechanics, with their tooltips",
            Self::Interface => {
                "interface labels, menus, system messages, and logs, which must be short"
            }
            Self::Lore => "books, cards, descriptions, and other lore, written as literature",
        }
    }
}

/// The domain of a sheet that is not dialogue, from its name. Quest and
/// cutscene sheets are dialogue; their lines have their own domains.
#[must_use]
pub fn sheet_domain(sheet: &str) -> Domain {
    if sheet.starts_with("quest/") || sheet.starts_with("cut_scene/") {
        return Domain::Dialogue;
    }
    let name = sheet
        .rsplit('/')
        .next()
        .unwrap_or(sheet)
        .to_ascii_lowercase();
    let has = |parts: &[&str]| parts.iter().any(|part| name.contains(part));
    if has(&[
        "npcname",
        "npcresident",
        "placename",
        "eobjname",
        "bnpcname",
        "title",
        "town",
        "worlddcgroup",
    ]) {
        Domain::Names
    } else if has(&["item", "gil", "currency", "shop", "recipe", "materia"]) {
        Domain::Items
    } else if has(&[
        "action",
        "trait",
        "status",
        "buff",
        "craftaction",
        "combo",
        "petaction",
        "generalaction",
    ]) {
        Domain::Actions
    } else if has(&[
        "card",
        "book",
        "fate",
        "leve",
        "transient",
        "description",
        "mount",
        "companion",
        "orchestrion",
        "fish",
        "achievement",
        "adventure",
        "notorious",
        "gathering",
        "journal",
    ]) {
        Domain::Lore
    } else {
        Domain::Interface
    }
}

/// One `## key` section of an agent Markdown file.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub key: String,
    /// Values of the `<!-- aeria: name=value; … -->` line under the heading.
    pub meta: BTreeMap<String, String>,
    pub text: String,
}

impl Section {
    #[must_use]
    pub fn new(key: &str, text: &str) -> Self {
        Self {
            key: key.trim().to_owned(),
            meta: BTreeMap::new(),
            text: text.trim().to_owned(),
        }
    }

    #[must_use]
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.meta.insert(name.to_owned(), value.trim().to_owned());
        self
    }
}

const META_START: &str = "<!-- aeria:";

fn parse_meta(line: &str) -> Option<BTreeMap<String, String>> {
    let inner = line.trim().strip_prefix(META_START)?.strip_suffix("-->")?;
    Some(
        inner
            .split(';')
            .filter_map(|pair| {
                let (name, value) = pair.split_once('=')?;
                Some((name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect(),
    )
}

/// Reads the `## key` sections of a Markdown file. Text before the first
/// heading is for people and is ignored; a later section with the same key
/// replaces an earlier one.
#[must_use]
pub fn parse_sections(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    let finish = |sections: &mut Vec<Section>, (key, lines): (String, Vec<&str>)| {
        let mut meta = BTreeMap::new();
        let mut body = Vec::new();
        for line in lines {
            match (
                body.iter().all(|line: &&str| line.trim().is_empty()),
                parse_meta(line),
            ) {
                (true, Some(values)) if meta.is_empty() => meta = values,
                _ => body.push(line),
            }
        }
        let text = body.join("\n").trim().to_owned();
        if key.is_empty() || text.is_empty() {
            return;
        }
        sections.retain(|section| !section.key.eq_ignore_ascii_case(&key));
        sections.push(Section { key, meta, text });
    };
    for line in text.trim_start_matches('\u{feff}').lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(done) = current.take() {
                finish(&mut sections, done);
            }
            current = Some((heading.trim().to_owned(), Vec::new()));
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    if let Some(done) = current.take() {
        finish(&mut sections, done);
    }
    sections
}

/// Writes sections in canonical form: a heading, the metadata line when
/// there is metadata, the text, and a blank line between sections.
#[must_use]
pub fn write_sections(title: &str, sections: &[Section]) -> String {
    let mut text = format!(
        "# {title}\n\nWritten by Aeria's agents. Edit freely; entries in the human files win.\n"
    );
    for section in sections {
        let _ = write!(text, "\n## {}\n", section.key);
        if !section.meta.is_empty() {
            let values: Vec<String> = section
                .meta
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            let _ = writeln!(text, "{META_START} {} -->", values.join("; "));
        }
        let _ = writeln!(text, "{}", section.text.trim());
    }
    text
}

/// The state of a lesson.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LessonStatus {
    /// Proposed from findings; used, and waiting for an evaluation.
    Trial,
    /// Kept after an evaluation or by a person.
    Active,
    /// Did not help; kept so it is not proposed again.
    Dropped,
}

impl LessonStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trial => "trial",
            Self::Active => "active",
            Self::Dropped => "dropped",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "active" => Self::Active,
            "dropped" => Self::Dropped,
            _ => Self::Trial,
        }
    }
}

/// A recurring problem and what to do instead.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lesson {
    pub id: String,
    pub status: LessonStatus,
    pub domain: Option<Domain>,
    pub text: String,
    pub meta: BTreeMap<String, String>,
}

impl Lesson {
    fn from_section(section: Section) -> Self {
        Self {
            id: section.key,
            status: LessonStatus::parse(section.meta.get("status").map_or("", String::as_str)),
            domain: section
                .meta
                .get("domain")
                .and_then(|domain| Domain::parse(domain)),
            text: section.text,
            meta: section.meta,
        }
    }

    #[must_use]
    pub fn to_section(&self) -> Section {
        let mut meta = self.meta.clone();
        meta.insert("status".to_owned(), self.status.as_str().to_owned());
        if let Some(domain) = self.domain {
            meta.insert("domain".to_owned(), domain.as_str().to_owned());
        }
        Section {
            key: self.id.clone(),
            meta,
            text: self.text.clone(),
        }
    }
}

/// A term with the layer it comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TermRef<'a> {
    pub entry: &'a GlossaryEntry,
    /// From the human glossary: agents never change it.
    pub locked: bool,
}

/// The project's knowledge from both layers.
#[derive(Clone, Debug, Default)]
pub struct Knowledge {
    /// The human layer.
    pub guide: ProjectGuide,
    pub style: Vec<Section>,
    pub terms: Vec<GlossaryEntry>,
    pub characters: VoiceProfiles,
    pub story: Vec<Section>,
    pub lessons: Vec<Lesson>,
    /// Why an agent file could not be used.
    pub problems: Vec<String>,
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_KNOWLEDGE_BYTES => {
            return Err(format!(
                "{} is larger than {MAX_KNOWLEDGE_BYTES} bytes",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    }
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    String::from_utf8(bytes)
        .map(|text| Some(text.trim_start_matches('\u{feff}').to_owned()))
        .map_err(|_| format!("{} is not UTF-8", path.display()))
}

/// The agent files' texts, for building knowledge without a repository.
#[derive(Clone, Debug, Default)]
pub struct AgentTexts {
    pub style: Option<String>,
    pub terms: Option<String>,
    pub characters: Option<String>,
    pub story: Option<String>,
    pub lessons: Option<String>,
}

impl Knowledge {
    /// Reads both layers from a repository root. Missing files are normal;
    /// unusable ones are reported in `problems` and left out.
    #[must_use]
    pub fn load(root: &Path) -> Self {
        let mut problems = Vec::new();
        let mut read = |file: KnowledgeFile| match read_text(&file.path(root)) {
            Ok(text) => text,
            Err(problem) => {
                problems.push(problem);
                None
            }
        };
        let texts = AgentTexts {
            style: read(KnowledgeFile::Style),
            terms: read(KnowledgeFile::Terms),
            characters: read(KnowledgeFile::Characters),
            story: read(KnowledgeFile::Story),
            lessons: read(KnowledgeFile::Lessons),
        };
        let mut knowledge = Self::from_parts(ProjectGuide::load(root), &texts);
        knowledge.problems.splice(0..0, problems);
        knowledge
    }

    /// Builds knowledge from the human layer and the agent files' texts.
    #[must_use]
    pub fn from_parts(guide: ProjectGuide, texts: &AgentTexts) -> Self {
        let mut problems = Vec::new();
        let terms = match texts
            .terms
            .as_deref()
            .map(|text| parse_glossary(text.as_bytes()))
        {
            Some(Ok(glossary)) => {
                for diagnostic in &glossary.diagnostics {
                    problems.push(format!(
                        "{KNOWLEDGE_DIR}/terms.csv line {}: {}",
                        diagnostic.line, diagnostic.message
                    ));
                }
                glossary.entries
            }
            Some(Err(error)) => {
                problems.push(format!("{KNOWLEDGE_DIR}/terms.csv: {error}"));
                Vec::new()
            }
            None => Vec::new(),
        };
        let characters = texts
            .characters
            .as_deref()
            .map(parse_voices)
            .unwrap_or_default();
        for diagnostic in &characters.diagnostics {
            problems.push(format!(
                "{KNOWLEDGE_DIR}/characters.md line {}: {}",
                diagnostic.line, diagnostic.message
            ));
        }
        let sections =
            |text: &Option<String>| text.as_deref().map(parse_sections).unwrap_or_default();
        Self {
            guide,
            style: sections(&texts.style),
            terms,
            characters,
            story: sections(&texts.story),
            lessons: sections(&texts.lessons)
                .into_iter()
                .map(Lesson::from_section)
                .collect(),
            problems,
        }
    }

    /// Terms that occur in `text`: human entries, then agent entries for
    /// terms the human glossary does not have.
    #[must_use]
    pub fn terms_in(&self, text: &str) -> Vec<TermRef<'_>> {
        let mut found: Vec<TermRef<'_>> = self
            .guide
            .glossary
            .as_ref()
            .map(|glossary| glossary.matches(text))
            .unwrap_or_default()
            .into_iter()
            .map(|entry| TermRef {
                entry,
                locked: true,
            })
            .collect();
        let agent = crate::guidance::Glossary {
            entries: self.terms.clone(),
            diagnostics: Vec::new(),
        };
        for entry in agent.matches(text) {
            if !found
                .iter()
                .any(|known| known.entry.term.eq_ignore_ascii_case(&entry.term))
                && let Some(entry) = self.terms.iter().find(|own| own.term == entry.term)
            {
                found.push(TermRef {
                    entry,
                    locked: false,
                });
            }
        }
        found
    }

    /// Whether either layer has a term, ignoring case.
    #[must_use]
    pub fn has_term(&self, term: &str) -> bool {
        let human = self.guide.glossary.as_ref().is_some_and(|glossary| {
            glossary
                .entries
                .iter()
                .any(|entry| entry.term.eq_ignore_ascii_case(term))
        });
        human
            || self
                .terms
                .iter()
                .any(|entry| entry.term.eq_ignore_ascii_case(term))
    }

    /// The profile of a speaker label, and whether it is locked.
    #[must_use]
    pub fn character(&self, speaker: &str) -> Option<(&VoiceProfile, bool)> {
        if let Some(profile) = self
            .guide
            .voices
            .as_ref()
            .and_then(|voices| voices.find(speaker))
        {
            return Some((profile, true));
        }
        self.characters
            .find(speaker)
            .map(|profile| (profile, false))
    }

    /// The style entries of a domain: the human guidance for
    /// [`Domain::General`], then the agents' entry.
    #[must_use]
    pub fn style(&self, domain: Domain) -> Option<&Section> {
        self.style
            .iter()
            .find(|section| section.key.eq_ignore_ascii_case(domain.as_str()))
    }

    #[must_use]
    pub fn story(&self, sheet: &str) -> Option<&Section> {
        self.story.iter().find(|section| section.key == sheet)
    }

    /// Domains among `domains` whose style no person has chosen yet: the
    /// agent entry is missing or was written by the study.
    #[must_use]
    pub fn uncalibrated(&self, domains: &[Domain]) -> Vec<Domain> {
        domains
            .iter()
            .copied()
            .filter(|domain| {
                self.style(*domain).is_none_or(|section| {
                    section
                        .meta
                        .get("source")
                        .is_none_or(|source| source == "study")
                })
            })
            .collect()
    }

    /// Lessons in use: active ones and those on trial.
    #[must_use]
    pub fn lessons_in_use(&self) -> Vec<&Lesson> {
        self.lessons
            .iter()
            .filter(|lesson| lesson.status != LessonStatus::Dropped)
            .collect()
    }

    /// The knowledge a unit of work needs, as prompt text: the guidance, the
    /// style of its domains, lessons, the terms in its sources, its
    /// speakers' profiles, and the stories of the given sheets.
    #[must_use]
    pub fn prompt_for<'a>(
        &self,
        domains: &[Domain],
        sources: impl IntoIterator<Item = &'a str>,
        speakers: impl IntoIterator<Item = &'a str>,
        stories: &[&str],
    ) -> String {
        let mut text = String::new();
        if let Some(guidance) = self.guide.guidance_for_prompt() {
            let _ = write!(
                text,
                "<guidance by the project's people>\n{}\n</guidance>\n",
                guidance.trim()
            );
        }
        let mut style = String::new();
        for domain in std::iter::once(Domain::General).chain(domains.iter().copied()) {
            if let Some(section) = self.style(domain)
                && !style.contains(&format!("## {}\n", domain.as_str()))
            {
                let _ = write!(
                    style,
                    "## {}\n{}\n",
                    domain.as_str(),
                    bounded(&section.text)
                );
            }
        }
        if !style.is_empty() {
            let _ = write!(text, "<style>\n{style}</style>\n");
        }
        let lessons: Vec<&Lesson> = self
            .lessons_in_use()
            .into_iter()
            .filter(|lesson| {
                lesson
                    .domain
                    .is_none_or(|domain| domain == Domain::General || domains.contains(&domain))
            })
            .take(MAX_PROMPT_LESSONS)
            .collect();
        if !lessons.is_empty() {
            text.push_str("<lessons learned in this project>\n");
            for lesson in lessons {
                let _ = writeln!(text, "- {}", bounded(&lesson.text).replace('\n', " "));
            }
            text.push_str("</lessons>\n");
        }
        let mut seen = Vec::new();
        let mut terms = String::new();
        for source in sources {
            for term in self.terms_in(source) {
                let key = term.entry.term.to_lowercase();
                if seen.len() >= MAX_PROMPT_TERMS || seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                let _ = write!(terms, "- {} → {}", term.entry.term, term.entry.translation);
                if let Some(note) = &term.entry.note {
                    let _ = write!(terms, " ({note})");
                }
                if !term.entry.forbidden.is_empty() {
                    let _ = write!(terms, "; never: {}", term.entry.forbidden.join(", "));
                }
                terms.push('\n');
            }
        }
        if !terms.is_empty() {
            let _ = write!(text, "<terms>\n{terms}</terms>\n");
        }
        let mut profiles: Vec<&VoiceProfile> = Vec::new();
        for speaker in speakers {
            if let Some((profile, _)) = self.character(speaker)
                && !profiles.iter().any(|known| std::ptr::eq(*known, profile))
                && profiles.len() < MAX_PROMPT_CHARACTERS
            {
                profiles.push(profile);
            }
        }
        if !profiles.is_empty() {
            text.push_str("<characters>\n");
            for profile in profiles {
                let _ = write!(
                    text,
                    "## {}\n{}\n\n",
                    profile.speakers.join(", "),
                    bounded(&strip_meta(&profile.text))
                );
            }
            text.push_str("</characters>\n");
        }
        let stories: Vec<&Section> = stories
            .iter()
            .filter_map(|sheet| self.story(sheet))
            .take(MAX_PROMPT_STORIES)
            .collect();
        if !stories.is_empty() {
            text.push_str("<story so far>\n");
            for story in stories {
                let _ = write!(text, "## {}\n{}\n\n", story.key, bounded(&story.text));
            }
            text.push_str("</story>\n");
        }
        text
    }
}

fn strip_meta(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with(META_START))
        .collect::<Vec<_>>()
        .join("\n")
}

fn bounded(text: &str) -> String {
    if text.chars().count() <= MAX_ENTRY_PROMPT_CHARS {
        return text.trim().to_owned();
    }
    let mut cut: String = text.chars().take(MAX_ENTRY_PROMPT_CHARS).collect();
    cut.push('…');
    cut
}

/// Replaces an agent file with `text`: written to a temporary file in the
/// same directory, synced, and renamed over the old one.
fn publish(root: &Path, file: KnowledgeFile, text: &str) -> Result<(), String> {
    let directory = root.join(KNOWLEDGE_DIR);
    fs::create_dir_all(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let path = file.path(root);
    let partial = directory.join(format!(".{}.partial", file.file_name()));
    let write = || -> std::io::Result<()> {
        let mut handle = fs::File::create(&partial)?;
        handle.write_all(text.as_bytes())?;
        handle.sync_all()?;
        fs::rename(&partial, &path)
    };
    write().map_err(|error| {
        let _ = fs::remove_file(&partial);
        format!("{}: {error}", path.display())
    })
}

/// Changes one agent file under the process-wide lock: reads its current
/// text, applies `change`, and publishes the result when it differs.
fn update(
    root: &Path,
    file: KnowledgeFile,
    change: impl FnOnce(Option<String>) -> Result<Option<String>, String>,
) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let current = read_text(&file.path(root))?;
    if let Some(next) = change(current.clone())?
        && Some(&next) != current.as_ref()
    {
        publish(root, file, &next)?;
    }
    Ok(())
}

/// Adds or replaces agent terms. Terms of the human glossary are skipped:
/// agents never change them. Returns the terms written.
///
/// # Errors
///
/// Returns a description when the terms file cannot be read or written.
pub fn set_terms(
    root: &Path,
    entries: &[GlossaryEntry],
    replace: bool,
) -> Result<Vec<String>, String> {
    let human = ProjectGuide::load(root)
        .glossary
        .map(|glossary| glossary.entries)
        .unwrap_or_default();
    let mut written = Vec::new();
    update(root, KnowledgeFile::Terms, |current| {
        let mut terms = match current
            .as_deref()
            .map(|text| parse_glossary(text.as_bytes()))
        {
            Some(Ok(glossary)) => glossary.entries,
            Some(Err(error)) => return Err(format!("{KNOWLEDGE_DIR}/terms.csv: {error}")),
            None => Vec::new(),
        };
        for entry in entries {
            let term = entry.term.trim();
            if term.is_empty()
                || entry.translation.trim().is_empty()
                || human
                    .iter()
                    .any(|known| known.term.eq_ignore_ascii_case(term))
            {
                continue;
            }
            let entry = GlossaryEntry {
                term: term.to_owned(),
                translation: entry.translation.trim().to_owned(),
                note: entry.note.clone(),
                forbidden: entry.forbidden.clone(),
            };
            match terms
                .iter_mut()
                .find(|known| known.term.eq_ignore_ascii_case(term))
            {
                Some(known) if replace => *known = entry,
                Some(_) => continue,
                None => terms.push(entry),
            }
            written.push(term.to_owned());
        }
        Ok((!written.is_empty()).then(|| write_glossary(&terms)))
    })?;
    Ok(written)
}

/// Sets agent character profiles. Speaker labels with a human profile are
/// left out: agents never change them. Returns the profiles written.
///
/// # Errors
///
/// Returns a description when the characters file cannot be read or
/// written.
pub fn set_characters(root: &Path, profiles: &[(Vec<String>, String)]) -> Result<usize, String> {
    let human = ProjectGuide::load(root).voices.unwrap_or_default();
    let usable: Vec<(Vec<String>, String)> = profiles
        .iter()
        .filter_map(|(labels, text)| {
            let labels: Vec<String> = labels
                .iter()
                .filter_map(|label| speaker_label(label))
                .filter(|label| human.find(label).is_none())
                .collect();
            (!labels.is_empty() && !text.trim().is_empty())
                .then(|| (labels, text.trim().to_owned()))
        })
        .collect();
    if usable.is_empty() {
        return Ok(0);
    }
    update(root, KnowledgeFile::Characters, |current| {
        let current = current.unwrap_or_else(|| {
            "# Characters\n\nWritten by Aeria's agents in Voice Profiles Format v1. Profiles in aeria-voices.md win.\n".to_owned()
        });
        // Labels of a new profile leave the profiles that had them.
        let parsed = parse_voices(&current);
        let remove: Vec<String> = usable
            .iter()
            .flat_map(|(labels, _)| labels.iter())
            .filter(|label| parsed.find(label).is_some())
            .cloned()
            .collect();
        change_voices(Some(&current), &usable, &remove).map(Some)
    })?;
    Ok(usable.len())
}

/// Sets one section of an agent Markdown file, replacing a section with the
/// same key.
///
/// # Errors
///
/// Returns a description when the file cannot be read or written.
pub fn set_section(root: &Path, file: KnowledgeFile, section: Section) -> Result<(), String> {
    let title = match file {
        KnowledgeFile::Style => "Style",
        KnowledgeFile::Story => "Story",
        KnowledgeFile::Lessons => "Lessons",
        KnowledgeFile::Terms | KnowledgeFile::Characters => {
            return Err("terms and characters are not sections".to_owned());
        }
    };
    if section.key.is_empty() || section.key.contains('\n') || section.text.trim().is_empty() {
        return Err("a section needs a one-line key and a text".to_owned());
    }
    update(root, file, |current| {
        let mut sections = current.as_deref().map(parse_sections).unwrap_or_default();
        match sections
            .iter_mut()
            .find(|known| known.key.eq_ignore_ascii_case(&section.key))
        {
            Some(known) => *known = section,
            None => sections.push(section),
        }
        Ok(Some(write_sections(title, &sections)))
    })
}

/// Sets a lesson.
///
/// # Errors
///
/// Returns a description when the lessons file cannot be read or written.
pub fn set_lesson(root: &Path, lesson: &Lesson) -> Result<(), String> {
    set_section(root, KnowledgeFile::Lessons, lesson.to_section())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn human() -> ProjectGuide {
        ProjectGuide::from_files(
            Ok(Some("Journal entries use вы.".to_owned())),
            Ok(Some("term,translation\nAether,Эфир\n".to_owned())),
        )
        .with_voices(Ok(Some("## URIANGER\nArchaic.\n".to_owned())))
    }

    #[test]
    fn sheets_are_sorted_into_domains() {
        assert_eq!(sheet_domain("quest/000/ClsCul021_00254"), Domain::Dialogue);
        assert_eq!(sheet_domain("BNpcName"), Domain::Names);
        assert_eq!(sheet_domain("PlaceName"), Domain::Names);
        assert_eq!(sheet_domain("Item"), Domain::Items);
        assert_eq!(sheet_domain("Action"), Domain::Actions);
        assert_eq!(sheet_domain("Status"), Domain::Actions);
        assert_eq!(sheet_domain("TripleTriadCard"), Domain::Lore);
        assert_eq!(sheet_domain("Addon"), Domain::Interface);
        assert_eq!(sheet_domain("LogMessage"), Domain::Interface);
    }

    #[test]
    fn sections_keep_metadata_and_the_last_duplicate() {
        let sections = parse_sections(
            "# Style\nfor people\n\n## journal\n<!-- aeria: source=study; updated=2026-09-29 -->\nUse вы.\n\n## Journal\nUse вы, neutrally.\n## empty\n\n## names\nTransliterate.",
        );
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].key, "Journal");
        assert_eq!(sections[0].text, "Use вы, neutrally.");
        assert_eq!(sections[1].key, "names");
        let written = write_sections(
            "Style",
            &[Section::new("journal", "Use вы.").with("source", "study")],
        );
        assert_eq!(
            parse_sections(&written),
            vec![Section::new("journal", "Use вы.").with("source", "study")]
        );
    }

    #[test]
    fn human_entries_win_and_the_prompt_takes_what_a_unit_needs() {
        let knowledge = Knowledge::from_parts(
            human(),
            &AgentTexts {
                style: Some("## journal\nUse вы.\n## items\nShort.\n## general\nUse «ёлочки».".to_owned()),
                terms: Some("term,translation\naether,Aether-agent\nFire Shard,огненный осколок\nMoogle,Моогл\n".to_owned()),
                characters: Some("## URIANGER\nModern.\n## LYNGSATH\nRough cook.\n".to_owned()),
                story: Some("## quest/000/A\nThe player met Lyngsath.".to_owned()),
                lessons: Some("## no-hm\n<!-- aeria: status=active -->\nDo not open lines with «Хм?».\n## bad\n<!-- aeria: status=dropped -->\nNever.\n## items-only\n<!-- aeria: status=trial; domain=items -->\nItems.".to_owned()),
            },
        );
        let terms = knowledge.terms_in("Bring a fire shard and some aether.");
        assert_eq!(terms.len(), 2);
        assert_eq!(
            (terms[0].entry.translation.as_str(), terms[0].locked),
            ("Эфир", true)
        );
        assert_eq!(
            (terms[1].entry.translation.as_str(), terms[1].locked),
            ("огненный осколок", false)
        );
        assert!(
            knowledge
                .character("urianger")
                .is_some_and(|(profile, locked)| locked && profile.text == "Archaic.")
        );
        assert!(
            knowledge
                .character("LYNGSATH")
                .is_some_and(|(_, locked)| !locked)
        );

        let prompt = knowledge.prompt_for(
            &[Domain::Journal, Domain::Dialogue],
            ["Bring a fire shard."],
            ["LYNGSATH"],
            &["quest/000/A"],
        );
        assert!(prompt.contains("Journal entries use вы."));
        assert!(prompt.contains("## general\nUse «ёлочки»."));
        assert!(prompt.contains("## journal\nUse вы."));
        assert!(!prompt.contains("Short."));
        assert!(prompt.contains("- Do not open lines with «Хм?»."));
        assert!(!prompt.contains("Never."));
        assert!(!prompt.contains("Items."));
        assert!(prompt.contains("- Fire Shard → огненный осколок"));
        assert!(!prompt.contains("Moogle"));
        assert!(prompt.contains("## LYNGSATH\nRough cook."));
        assert!(prompt.contains("The player met Lyngsath."));
    }

    #[test]
    fn agent_writes_keep_human_entries_and_each_other() {
        let directory = tempfile::tempdir().expect("temp");
        let root = directory.path();
        fs::write(
            root.join("aeria-glossary.csv"),
            "term,translation\nAether,Эфир\n",
        )
        .expect("glossary");
        fs::write(root.join("aeria-voices.md"), "## URIANGER\nArchaic.\n").expect("voices");

        let entry = |term: &str, translation: &str| GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            note: None,
            forbidden: Vec::new(),
        };
        let written = set_terms(
            root,
            &[
                entry("aether", "x"),
                entry("Fire Shard", "огненный осколок"),
            ],
            false,
        )
        .expect("terms");
        assert_eq!(written, vec!["Fire Shard"]);
        assert!(
            set_terms(root, &[entry("fire shard", "другое")], false)
                .expect("again")
                .is_empty()
        );
        assert_eq!(
            set_terms(root, &[entry("fire shard", "осколок огня")], true).expect("replace"),
            vec!["fire shard"]
        );

        let profiles = vec![(
            vec!["URIANGER".to_owned(), "LYNGSATH".to_owned()],
            "Rough.".to_owned(),
        )];
        assert_eq!(set_characters(root, &profiles).expect("characters"), 1);
        set_section(
            root,
            KnowledgeFile::Style,
            Section::new("journal", "Use вы."),
        )
        .expect("style");
        set_section(
            root,
            KnowledgeFile::Style,
            Section::new("journal", "Use вы, always."),
        )
        .expect("style again");
        set_lesson(
            root,
            &Lesson {
                id: "no-hm".to_owned(),
                status: LessonStatus::Trial,
                domain: Some(Domain::Dialogue),
                text: "Do not open lines with «Хм?».".to_owned(),
                meta: BTreeMap::new(),
            },
        )
        .expect("lesson");

        let knowledge = Knowledge::load(root);
        assert!(knowledge.problems.is_empty(), "{:?}", knowledge.problems);
        assert_eq!(knowledge.terms.len(), 1);
        assert_eq!(knowledge.terms[0].translation, "осколок огня");
        assert!(knowledge.characters.find("URIANGER").is_none());
        assert!(
            knowledge
                .character("LYNGSATH")
                .is_some_and(|(profile, _)| profile.text == "Rough.")
        );
        assert_eq!(
            knowledge
                .style(Domain::Journal)
                .map(|section| section.text.as_str()),
            Some("Use вы, always.")
        );
        assert_eq!(knowledge.lessons[0].domain, Some(Domain::Dialogue));
        assert_eq!(knowledge.lessons[0].status, LessonStatus::Trial);
        assert!(!root.join(KNOWLEDGE_DIR).join(".terms.csv.partial").exists());
    }
}
