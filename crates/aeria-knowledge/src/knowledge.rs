//! Loading the knowledge files and picking what one scene needs.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::glossary::{Glossary, GlossaryEntry, parse_glossary};
use crate::sections::{Section, parse_sections};
use crate::voices::{VoiceProfile, VoiceProfiles, parse_voices};

/// The knowledge directory at the project root.
pub const KNOWLEDGE_DIR: &str = "aeria-knowledge";
/// Largest knowledge file read.
pub const MAX_KNOWLEDGE_BYTES: u64 = 8 * 1024 * 1024;
/// Longest text of one entry placed in a scene's knowledge.
const MAX_ENTRY_CHARS: usize = 2_500;
/// Most terms placed in a scene's knowledge.
const MAX_SCENE_TERMS: usize = 120;
/// Most character profiles placed in a scene's knowledge.
const MAX_SCENE_CHARACTERS: usize = 16;
/// Most lessons placed in a scene's knowledge.
const MAX_SCENE_LESSONS: usize = 30;

/// A knowledge file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KnowledgeFile {
    Style,
    Terms,
    Characters,
    Story,
    Lessons,
}

impl KnowledgeFile {
    pub const ALL: [Self; 5] = [
        Self::Style,
        Self::Terms,
        Self::Characters,
        Self::Story,
        Self::Lessons,
    ];

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

    /// The path relative to the project root, with `/`.
    #[must_use]
    pub fn relative_path(self) -> String {
        format!("{KNOWLEDGE_DIR}/{}", self.file_name())
    }

    #[must_use]
    pub fn path(self, root: &Path) -> PathBuf {
        root.join(KNOWLEDGE_DIR).join(self.file_name())
    }
}

impl KnowledgeFile {
    /// The text of the file in a new project: a title and what the file
    /// holds, with no entries.
    #[must_use]
    pub fn empty(self) -> String {
        match self {
            Self::Style => {
                "# Style\n\nHow each kind of text reads, one `## <kind>` section per kind: \
                            general, journal, objective, system, dialogue, names, items, actions, \
                            interface, lore.\n"
                    .to_owned()
            }
            Self::Terms => crate::glossary::write_glossary(&[]),
            Self::Characters => "# Characters\n\nHow characters speak, one `## LABEL` section per \
                                 character, named by the speaker labels of the game's text.\n"
                .to_owned(),
            Self::Story => "# Story\n\nWhat happened so far, one `## <sheet>` section per quest \
                            or cutscene sheet.\n"
                .to_owned(),
            Self::Lessons => "# Lessons\n\nRecurring problems and what to do instead, one \
                              `## <id>` section per lesson.\n"
                .to_owned(),
        }
    }
}

/// Writes every knowledge file the project does not have yet, with no
/// entries. Returns the files written, relative to the root.
///
/// # Errors
///
/// Returns a description when a file cannot be written.
pub fn create_empty(root: &Path) -> Result<Vec<String>, String> {
    let directory = root.join(KNOWLEDGE_DIR);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut written = Vec::new();
    for file in KnowledgeFile::ALL {
        let path = file.path(root);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, file.empty())
            .map_err(|error| format!("{}: {error}", path.display()))?;
        written.push(file.relative_path());
    }
    Ok(written)
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

    /// What the domain covers.
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

/// A recurring problem and what to do instead.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lesson {
    pub id: String,
    /// The domain the lesson applies to; `None` for all text.
    pub domain: Option<Domain>,
    pub text: String,
    pub settled: bool,
}

/// The project's knowledge.
#[derive(Clone, Debug, Default)]
pub struct Knowledge {
    /// One section per domain.
    pub style: Vec<Section>,
    pub terms: Glossary,
    pub characters: VoiceProfiles,
    /// One section per quest or cutscene sheet.
    pub story: Vec<Section>,
    pub lessons: Vec<Lesson>,
    /// Why a file or an entry could not be used, with the file and line.
    pub problems: Vec<String>,
}

/// The files' texts, for building knowledge without a project.
#[derive(Clone, Debug, Default)]
pub struct KnowledgeTexts {
    pub style: Option<String>,
    pub terms: Option<String>,
    pub characters: Option<String>,
    pub story: Option<String>,
    pub lessons: Option<String>,
}

/// Reads one knowledge file as text. A missing file is `None`.
///
/// # Errors
///
/// Returns a description when the file is too large, unreadable, or not
/// UTF-8.
pub fn read_file(root: &Path, file: KnowledgeFile) -> Result<Option<String>, String> {
    let path = file.path(root);
    let name = file.relative_path();
    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_KNOWLEDGE_BYTES => {
            return Err(format!("{name} is larger than {MAX_KNOWLEDGE_BYTES} bytes"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{name}: {error}")),
    }
    let bytes = fs::read(&path).map_err(|error| format!("{name}: {error}"))?;
    String::from_utf8(bytes)
        .map(|text| Some(text.trim_start_matches('\u{feff}').to_owned()))
        .map_err(|_| format!("{name} is not UTF-8"))
}

impl Knowledge {
    /// Reads the knowledge of a project. Missing files are normal; unusable
    /// files and entries are reported in `problems` and left out.
    #[must_use]
    pub fn load(root: &Path) -> Self {
        let mut problems = Vec::new();
        let mut read = |file: KnowledgeFile| match read_file(root, file) {
            Ok(text) => text,
            Err(problem) => {
                problems.push(problem);
                None
            }
        };
        let texts = KnowledgeTexts {
            style: read(KnowledgeFile::Style),
            terms: read(KnowledgeFile::Terms),
            characters: read(KnowledgeFile::Characters),
            story: read(KnowledgeFile::Story),
            lessons: read(KnowledgeFile::Lessons),
        };
        let mut knowledge = Self::from_texts(&texts);
        knowledge.problems.splice(0..0, problems);
        knowledge
    }

    /// Builds knowledge from the files' texts.
    #[must_use]
    pub fn from_texts(texts: &KnowledgeTexts) -> Self {
        let mut problems = Vec::new();
        let terms_name = KnowledgeFile::Terms.relative_path();
        let terms = match texts
            .terms
            .as_deref()
            .map(|text| parse_glossary(text.as_bytes()))
        {
            Some(Ok(glossary)) => {
                for diagnostic in &glossary.diagnostics {
                    problems.push(format!(
                        "{terms_name} line {}: {}",
                        diagnostic.line, diagnostic.message
                    ));
                }
                glossary
            }
            Some(Err(error)) => {
                problems.push(error.to_string());
                Glossary::default()
            }
            None => Glossary::default(),
        };
        let characters = texts
            .characters
            .as_deref()
            .map(parse_voices)
            .unwrap_or_default();
        for diagnostic in &characters.diagnostics {
            problems.push(format!(
                "{} line {}: {}",
                KnowledgeFile::Characters.relative_path(),
                diagnostic.line,
                diagnostic.message
            ));
        }
        let sections =
            |text: &Option<String>| text.as_deref().map(parse_sections).unwrap_or_default();
        let style = sections(&texts.style);
        for section in &style {
            if Domain::parse(&section.key).is_none() {
                problems.push(format!(
                    "{} line {}: {:?} is not a kind of text; the keys are {}",
                    KnowledgeFile::Style.relative_path(),
                    section.line,
                    section.key,
                    Domain::ALL.map(Domain::as_str).join(", ")
                ));
            }
        }
        let lessons = sections(&texts.lessons)
            .into_iter()
            .map(|section| {
                let domain = section.meta.get("domain").and_then(|name| {
                    let domain = Domain::parse(name);
                    if domain.is_none() {
                        problems.push(format!(
                            "{} line {}: {name:?} is not a kind of text",
                            KnowledgeFile::Lessons.relative_path(),
                            section.line
                        ));
                    }
                    domain
                });
                Lesson {
                    settled: section.settled(),
                    id: section.key,
                    domain,
                    text: section.text,
                }
            })
            .collect();
        Self {
            style,
            terms,
            characters,
            story: sections(&texts.story),
            lessons,
            problems,
        }
    }

    /// Terms that occur in `text`.
    #[must_use]
    pub fn terms_in(&self, text: &str) -> Vec<&GlossaryEntry> {
        self.terms.matches(text)
    }

    /// The profile of a speaker label.
    #[must_use]
    pub fn character(&self, speaker: &str) -> Option<&VoiceProfile> {
        self.characters.find(speaker)
    }

    /// The style of a domain.
    #[must_use]
    pub fn style(&self, domain: Domain) -> Option<&Section> {
        self.style
            .iter()
            .find(|section| section.key.eq_ignore_ascii_case(domain.as_str()))
    }

    /// The story of a quest or cutscene sheet.
    #[must_use]
    pub fn story(&self, sheet: &str) -> Option<&Section> {
        self.story.iter().find(|section| section.key == sheet)
    }

    /// Lessons for text of some domains: those without a domain and those
    /// of one of `domains`.
    #[must_use]
    pub fn lessons_for(&self, domains: &[Domain]) -> Vec<&Lesson> {
        self.lessons
            .iter()
            .filter(|lesson| {
                lesson
                    .domain
                    .is_none_or(|domain| domain == Domain::General || domains.contains(&domain))
            })
            .collect()
    }

    /// The knowledge a scene needs, as Markdown: the style of its domains,
    /// lessons, the terms in its sources, its speakers' profiles, and the
    /// stories of the given sheets. Settled entries are marked.
    #[must_use]
    pub fn scene_slice<'a>(
        &self,
        domains: &[Domain],
        sources: impl IntoIterator<Item = &'a str>,
        speakers: impl IntoIterator<Item = &'a str>,
        stories: &[&str],
    ) -> String {
        let settled = |yes: bool| if yes { " (settled)" } else { "" };
        let mut text = String::new();
        let mut seen_domains = Vec::new();
        for domain in std::iter::once(Domain::General).chain(domains.iter().copied()) {
            if seen_domains.contains(&domain) {
                continue;
            }
            seen_domains.push(domain);
            if let Some(section) = self.style(domain) {
                let _ = write!(
                    text,
                    "### Style: {}{}\n{}\n\n",
                    domain.as_str(),
                    settled(section.settled()),
                    bounded(&section.text)
                );
            }
        }
        let lessons: Vec<&Lesson> = self
            .lessons_for(domains)
            .into_iter()
            .take(MAX_SCENE_LESSONS)
            .collect();
        if !lessons.is_empty() {
            text.push_str("### Lessons\n");
            for lesson in lessons {
                let _ = writeln!(
                    text,
                    "- {}{}",
                    bounded(&lesson.text).replace('\n', " "),
                    settled(lesson.settled)
                );
            }
            text.push('\n');
        }
        let mut seen = Vec::new();
        let mut terms = String::new();
        for source in sources {
            for entry in self.terms_in(source) {
                let key = entry.term.to_lowercase();
                if seen.len() >= MAX_SCENE_TERMS || seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                let _ = write!(terms, "- {} → {}", entry.term, entry.translation);
                if let Some(note) = &entry.note {
                    let _ = write!(terms, " ({note})");
                }
                if !entry.forbidden.is_empty() {
                    let _ = write!(terms, "; never: {}", entry.forbidden.join(", "));
                }
                terms.push_str(settled(entry.settled));
                terms.push('\n');
            }
        }
        if !terms.is_empty() {
            let _ = write!(text, "### Terms\n{terms}\n");
        }
        let mut profiles: Vec<&VoiceProfile> = Vec::new();
        for speaker in speakers {
            if let Some(profile) = self.character(speaker)
                && !profiles.iter().any(|known| std::ptr::eq(*known, profile))
                && profiles.len() < MAX_SCENE_CHARACTERS
            {
                profiles.push(profile);
            }
        }
        for profile in profiles {
            let _ = write!(
                text,
                "### Voice: {}{}\n{}\n\n",
                profile.speakers.join(", "),
                settled(profile.settled),
                bounded(&profile.text)
            );
        }
        for sheet in stories {
            if let Some(story) = self.story(sheet) {
                let _ = write!(
                    text,
                    "### Story so far: {}\n{}\n\n",
                    story.key,
                    bounded(&story.text)
                );
            }
        }
        text.trim_end().to_owned()
    }
}

fn bounded(text: &str) -> String {
    if text.chars().count() <= MAX_ENTRY_CHARS {
        return text.trim().to_owned();
    }
    let mut cut: String = text.chars().take(MAX_ENTRY_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {

    #[test]
    fn empty_files_read_without_problems_and_are_never_replaced() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        assert_eq!(super::create_empty(root).expect("create").len(), 5);
        let knowledge = Knowledge::load(root);
        assert!(knowledge.problems.is_empty(), "{:?}", knowledge.problems);
        assert!(knowledge.terms.entries.is_empty());
        std::fs::write(
            KnowledgeFile::Story.path(root),
            "## a
kept
",
        )
        .expect("write");
        assert!(super::create_empty(root).expect("again").is_empty());
        assert_eq!(
            std::fs::read_to_string(KnowledgeFile::Story.path(root)).expect("read"),
            "## a
kept
"
        );
    }

    use super::*;

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

    fn texts() -> KnowledgeTexts {
        KnowledgeTexts {
            style: Some("## general\n<!-- aeria: settled=yes -->\nUse «ёлочки».\n## journal\nUse вы.\n## items\nShort.\n## weather\nNo.".to_owned()),
            terms: Some("term,translation,forbidden,settled\nAether,Эфир,Этер,yes\nFire Shard,огненный осколок,,\nMoogle,Моогл,,\n,broken,,\n".to_owned()),
            characters: Some("## LYNGSATH\nRough cook.\n".to_owned()),
            story: Some("## quest/000/A\nThe player met Lyngsath.".to_owned()),
            lessons: Some("## no-hm\nDo not open lines with «Хм?».\n## items-only\n<!-- aeria: domain=items -->\nItems.\n## odd\n<!-- aeria: domain=weather -->\nOdd.".to_owned()),
        }
    }

    #[test]
    fn a_scene_gets_the_knowledge_it_needs() {
        let knowledge = Knowledge::from_texts(&texts());
        assert_eq!(knowledge.problems.len(), 3, "{:?}", knowledge.problems);
        let slice = knowledge.scene_slice(
            &[Domain::Journal, Domain::Dialogue],
            ["Bring a fire shard and some aether."],
            ["LYNGSATH"],
            &["quest/000/A"],
        );
        assert!(slice.contains("### Style: general (settled)\nUse «ёлочки»."));
        assert!(slice.contains("### Style: journal\nUse вы."));
        assert!(!slice.contains("Short."));
        assert!(slice.contains("- Do not open lines with «Хм?»."));
        assert!(!slice.contains("Items."));
        assert!(slice.contains("- Aether → Эфир; never: Этер (settled)"));
        assert!(slice.contains("- Fire Shard → огненный осколок\n"));
        assert!(!slice.contains("Moogle"));
        assert!(slice.contains("### Voice: LYNGSATH\nRough cook."));
        assert!(slice.contains("The player met Lyngsath."));
    }

    #[test]
    fn files_are_read_from_the_knowledge_directory() {
        let directory = tempfile::tempdir().expect("temp");
        let root = directory.path();
        assert!(Knowledge::load(root).problems.is_empty());
        fs::create_dir(root.join(KNOWLEDGE_DIR)).expect("dir");
        fs::write(
            KnowledgeFile::Terms.path(root),
            "\u{feff}term,translation\nAether,Эфир\n",
        )
        .expect("terms");
        fs::write(KnowledgeFile::Style.path(root), [0xff, 0xfe]).expect("style");
        let knowledge = Knowledge::load(root);
        assert_eq!(knowledge.terms.entries.len(), 1);
        assert_eq!(
            knowledge.problems,
            ["aeria-knowledge/style.md is not UTF-8"]
        );
    }
}
