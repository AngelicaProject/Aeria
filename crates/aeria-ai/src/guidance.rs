//! Project guidance and glossary: human-edited files at the repository root.
//!
//! `aeria-guidance.md` is free-form Markdown passed to the model as text.
//! `aeria-glossary.csv` is Glossary Format v1 (`docs/formats/glossary-v1.md`).
//! Both are optional and shared through Git like any other project file.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Project guidance file name at the repository root.
pub const GUIDANCE_FILE: &str = "aeria-guidance.md";
/// Glossary file name at the repository root.
pub const GLOSSARY_FILE: &str = "aeria-glossary.csv";
/// Largest guidance file read.
pub const MAX_GUIDANCE_BYTES: u64 = 64 * 1024;
/// Largest glossary file read.
pub const MAX_GLOSSARY_BYTES: u64 = 2 * 1024 * 1024;
/// Most glossary entries read.
pub const MAX_GLOSSARY_ENTRIES: usize = 20_000;
/// Longest guidance text placed in a request.
pub const MAX_GUIDANCE_PROMPT_CHARS: usize = 12_000;

const COLUMNS: [&str; 4] = ["term", "translation", "note", "forbidden"];

/// A project-shared file Angelica may propose changes to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectFile {
    Guidance,
    Glossary,
}

impl ProjectFile {
    /// The file name at the repository root.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Guidance => GUIDANCE_FILE,
            Self::Glossary => GLOSSARY_FILE,
        }
    }

    /// The largest accepted file.
    #[must_use]
    pub const fn max_bytes(self) -> u64 {
        match self {
            Self::Guidance => MAX_GUIDANCE_BYTES,
            Self::Glossary => MAX_GLOSSARY_BYTES,
        }
    }
}

/// Reads one project file as text. A missing file is `None`.
///
/// # Errors
///
/// Returns a description when the file is too large, unreadable, or not
/// UTF-8.
pub fn read_project_file(
    repository_root: &Path,
    file: ProjectFile,
) -> Result<Option<String>, String> {
    let path = repository_root.join(file.file_name());
    match read_bounded(&path, file.max_bytes())? {
        Some(bytes) => String::from_utf8(bytes)
            .map(|text| Some(text.trim_start_matches('\u{feff}').to_owned()))
            .map_err(|_| format!("{} is not UTF-8", file.file_name())),
        None => Ok(None),
    }
}

/// Applies glossary additions and removals and returns the new file in
/// canonical form. An added term replaces an entry with the same term.
///
/// # Errors
///
/// Returns a description when the current file cannot be used or has invalid
/// rows, which a rewrite would drop, or when an addition is invalid.
pub fn change_glossary(
    current: Option<&str>,
    add: Vec<GlossaryEntry>,
    remove: &[String],
) -> Result<String, String> {
    let mut entries = match current {
        Some(text) => {
            let glossary = parse_glossary(text.as_bytes()).map_err(|error| error.to_string())?;
            if let Some(diagnostic) = glossary.diagnostics.first() {
                return Err(format!(
                    "{GLOSSARY_FILE} has invalid rows (line {}: {}); they must be fixed by hand before Angelica can change the glossary",
                    diagnostic.line, diagnostic.message
                ));
            }
            glossary.entries
        }
        None => Vec::new(),
    };
    for term in remove {
        let term = term.trim().to_lowercase();
        entries.retain(|entry| entry.term.to_lowercase() != term);
    }
    for mut entry in add {
        let (term, translation) = (
            entry.term.trim().to_owned(),
            entry.translation.trim().to_owned(),
        );
        entry.term = term;
        entry.translation = translation;
        if entry.term.is_empty() || entry.translation.is_empty() {
            return Err("a glossary entry needs a term and a translation".to_owned());
        }
        let key = entry.term.to_lowercase();
        match entries
            .iter_mut()
            .find(|existing| existing.term.to_lowercase() == key)
        {
            Some(existing) => *existing = entry,
            None => entries.push(entry),
        }
    }
    if entries.len() > MAX_GLOSSARY_ENTRIES {
        return Err(format!(
            "the glossary would exceed {MAX_GLOSSARY_ENTRIES} entries"
        ));
    }
    Ok(write_glossary(&entries))
}

/// One glossary entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub term: String,
    pub translation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Translations that must not be used for the term.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forbidden: Vec<String>,
}

/// A glossary row that was excluded, with the reason.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryDiagnostic {
    /// One-based line of the record in the file.
    pub line: u64,
    pub message: String,
}

/// A parsed glossary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Glossary {
    pub entries: Vec<GlossaryEntry>,
    pub diagnostics: Vec<GlossaryDiagnostic>,
}

/// A glossary file that cannot be read at all.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{GLOSSARY_FILE} cannot be used: {message}")]
pub struct GlossaryError {
    pub message: String,
}

fn glossary_error(message: impl Into<String>) -> GlossaryError {
    GlossaryError {
        message: message.into(),
    }
}

/// Parses Glossary Format v1.
///
/// Invalid rows are excluded and reported; the file is refused only when its
/// header is invalid, it is not UTF-8 CSV, or it is too large.
///
/// # Errors
///
/// Returns [`GlossaryError`] for a file that cannot be used at all.
pub fn parse_glossary(bytes: &[u8]) -> Result<Glossary, GlossaryError> {
    if bytes.len() as u64 > MAX_GLOSSARY_BYTES {
        return Err(glossary_error(format!(
            "the file is larger than {MAX_GLOSSARY_BYTES} bytes"
        )));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|error| glossary_error(format!("the header cannot be read: {error}")))?
        .clone();
    let mut index = [None; 4];
    for (position, name) in headers.iter().enumerate() {
        let name = name.trim();
        let Some(column) = COLUMNS.iter().position(|column| *column == name) else {
            return Err(glossary_error(format!(
                "unknown column {name:?}; the columns are {}",
                COLUMNS.join(", ")
            )));
        };
        if index[column].replace(position).is_some() {
            return Err(glossary_error(format!("column {name:?} appears twice")));
        }
    }
    if index[0].is_none() || index[1].is_none() {
        return Err(glossary_error(
            "the header must name the columns term and translation",
        ));
    }
    let mut glossary = Glossary::default();
    let mut seen = std::collections::HashSet::new();
    for record in reader.records() {
        let record = record.map_err(|error| glossary_error(format!("invalid CSV: {error}")))?;
        let line = record.position().map_or(0, csv::Position::line);
        let field = |column: usize| {
            index[column]
                .and_then(|position| record.get(position))
                .map(str::trim)
                .unwrap_or_default()
        };
        if record.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        let mut reject = |message: String| {
            glossary
                .diagnostics
                .push(GlossaryDiagnostic { line, message });
        };
        let term = field(0);
        let translation = field(1);
        if term.is_empty() || translation.is_empty() {
            reject("the term and its translation must not be empty".to_owned());
            continue;
        }
        if !seen.insert(term.to_lowercase()) {
            reject(format!("the term {term:?} is already defined above"));
            continue;
        }
        if glossary.entries.len() >= MAX_GLOSSARY_ENTRIES {
            reject(format!(
                "only the first {MAX_GLOSSARY_ENTRIES} entries are used"
            ));
            break;
        }
        glossary.entries.push(GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            note: Some(field(2))
                .filter(|note| !note.is_empty())
                .map(str::to_owned),
            forbidden: field(3)
                .split(';')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
        });
    }
    Ok(glossary)
}

/// Writes entries in the canonical Glossary Format v1 form: the full header,
/// one entry per line, LF line endings, and quoting only where needed.
#[must_use]
pub fn write_glossary(entries: &[GlossaryEntry]) -> String {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    let _ = writer.write_record(COLUMNS);
    for entry in entries {
        let forbidden = entry.forbidden.join("; ");
        let _ = writer.write_record([
            entry.term.as_str(),
            entry.translation.as_str(),
            entry.note.as_deref().unwrap_or_default(),
            forbidden.as_str(),
        ]);
    }
    String::from_utf8(writer.into_inner().unwrap_or_default()).unwrap_or_default()
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Finds `needle` in `haystack` case-insensitively, as a whole word when it
/// starts and ends with word characters.
fn contains_term(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let starts_word = needle.chars().next().is_some_and(is_word);
    let ends_word = needle.chars().last().is_some_and(is_word);
    haystack.match_indices(&needle).any(|(start, _)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + needle.len()..].chars().next();
        (!starts_word || !before.is_some_and(is_word))
            && (!ends_word || !after.is_some_and(is_word))
    })
}

/// Whether every word of `translation` appears in `target`, allowing for
/// inflected endings: each word's leading two thirds must be present.
fn contains_inflected(target: &str, translation: &str) -> bool {
    let target = target.to_lowercase();
    translation.to_lowercase().split_whitespace().all(|word| {
        let characters: Vec<char> = word.chars().collect();
        let stem_length = (characters.len() * 2)
            .div_ceil(3)
            .max(3)
            .min(characters.len());
        let stem: String = characters[..stem_length].iter().collect();
        target.contains(&stem)
    })
}

impl Glossary {
    /// Entries whose term occurs in `text`.
    #[must_use]
    pub fn matches(&self, text: &str) -> Vec<&GlossaryEntry> {
        self.entries
            .iter()
            .filter(|entry| contains_term(text, &entry.term))
            .collect()
    }

    /// Entries whose term contains one of `queries`, case-insensitively.
    #[must_use]
    pub fn lookup(&self, queries: &[String]) -> Vec<&GlossaryEntry> {
        let queries: Vec<String> = queries
            .iter()
            .map(|query| query.trim().to_lowercase())
            .collect();
        self.entries
            .iter()
            .filter(|entry| {
                let term = entry.term.to_lowercase();
                queries
                    .iter()
                    .any(|query| !query.is_empty() && term.contains(query))
            })
            .collect()
    }

    /// Advisory findings for a translation: glossary translations that seem
    /// missing and forbidden variants that are used. They never block a
    /// write, since inflected languages rarely keep the dictionary form.
    #[must_use]
    pub fn check(&self, source: &str, target: &str) -> Vec<String> {
        let mut findings = Vec::new();
        for entry in self.matches(source) {
            if !contains_inflected(target, &entry.translation) {
                findings.push(format!(
                    "the glossary translates {:?} as {:?}, which does not seem to be used",
                    entry.term, entry.translation
                ));
            }
            for forbidden in &entry.forbidden {
                if contains_term(target, forbidden) {
                    findings.push(format!(
                        "the glossary forbids {forbidden:?} for {:?}; use {:?}",
                        entry.term, entry.translation
                    ));
                }
            }
        }
        findings
    }
}

/// The project's guidance and glossary as read from the repository.
#[derive(Clone, Debug, Default)]
pub struct ProjectGuide {
    pub guidance: Option<String>,
    pub glossary: Option<Glossary>,
    /// Why an existing file could not be used.
    pub problems: Vec<String>,
}

fn read_bounded(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > limit => {
            return Err(format!("{} is larger than {limit} bytes", path.display()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    }
    fs::read(path)
        .map(Some)
        .map_err(|error| format!("{}: {error}", path.display()))
}

impl ProjectGuide {
    /// Reads both files from a repository root. Missing files are normal;
    /// unusable ones are reported in `problems`.
    #[must_use]
    pub fn load(repository_root: &Path) -> Self {
        Self::from_files(
            read_project_file(repository_root, ProjectFile::Guidance),
            read_project_file(repository_root, ProjectFile::Glossary),
        )
    }

    /// Builds the guide from the two files' contents as read.
    #[must_use]
    pub fn from_files(
        guidance: Result<Option<String>, String>,
        glossary: Result<Option<String>, String>,
    ) -> Self {
        let mut guide = Self::default();
        match guidance {
            Ok(Some(text)) => {
                let text = text.trim().to_owned();
                guide.guidance = (!text.is_empty()).then_some(text);
            }
            Ok(None) => {}
            Err(problem) => guide.problems.push(problem),
        }
        match glossary {
            Ok(Some(text)) => match parse_glossary(text.as_bytes()) {
                Ok(glossary) => guide.glossary = Some(glossary),
                Err(error) => guide.problems.push(error.to_string()),
            },
            Ok(None) => {}
            Err(problem) => guide.problems.push(problem),
        }
        guide
    }

    /// Guidance text bounded for a request.
    #[must_use]
    pub fn guidance_for_prompt(&self) -> Option<String> {
        let text = self.guidance.as_ref()?;
        if text.chars().count() <= MAX_GUIDANCE_PROMPT_CHARS {
            return Some(text.clone());
        }
        let cut: String = text.chars().take(MAX_GUIDANCE_PROMPT_CHARS).collect();
        Some(format!(
            "{cut}\n[guidance truncated; read the rest with get_guidance]"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str, translation: &str) -> GlossaryEntry {
        GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            note: None,
            forbidden: Vec::new(),
        }
    }

    #[test]
    fn glossary_rows_are_read_and_invalid_rows_reported() {
        let csv = "\u{feff}translation,term,forbidden\nЭфир,Aether,Этер; Эфиръ\n,Empty,\nКристалл,Crystal,\nКристал,crystal,\n\n\"Мудрец, старший\",\"Sage, elder\",\n";
        let glossary = parse_glossary(csv.as_bytes()).expect("glossary");
        assert_eq!(glossary.entries.len(), 3);
        assert_eq!(glossary.entries[0].forbidden, vec!["Этер", "Эфиръ"]);
        assert_eq!(glossary.entries[2].term, "Sage, elder");
        assert_eq!(glossary.diagnostics.len(), 2);
        assert_eq!(glossary.diagnostics[0].line, 3);
        assert!(glossary.diagnostics[1].message.contains("already defined"));
    }

    #[test]
    fn a_bad_header_refuses_the_file() {
        assert!(parse_glossary(b"term,meaning\na,b\n").is_err());
        assert!(parse_glossary(b"term\na\n").is_err());
        assert!(parse_glossary(b"term,term,translation\n").is_err());
        assert!(
            parse_glossary(b"term,translation\n")
                .expect("empty")
                .entries
                .is_empty()
        );
    }

    #[test]
    fn the_canonical_writer_round_trips() {
        let entries = vec![
            GlossaryEntry {
                note: Some("line \"one\"".to_owned()),
                forbidden: vec!["Этер".to_owned()],
                ..entry("Aether", "Эфир")
            },
            entry("Sage, elder", "Мудрец"),
        ];
        let text = write_glossary(&entries);
        assert!(text.starts_with("term,translation,note,forbidden\n"));
        assert!(!text.contains('\r'));
        assert_eq!(
            parse_glossary(text.as_bytes()).expect("parse").entries,
            entries
        );
    }

    #[test]
    fn terms_match_whole_words_case_insensitively() {
        let glossary = Glossary {
            entries: vec![
                entry("Aether", "Эфир"),
                entry("Ul'dah", "Ул'да"),
                entry("+1", "+1"),
            ],
            diagnostics: Vec::new(),
        };
        let found: Vec<&str> = glossary
            .matches("The AETHER of Ul'dah. Aetherial +1")
            .iter()
            .map(|entry| entry.term.as_str())
            .collect();
        assert_eq!(found, vec!["Aether", "Ul'dah", "+1"]);
        assert!(glossary.matches("Aetherial only").is_empty());
        assert_eq!(glossary.lookup(&["ul'".to_owned()]).len(), 1);
    }

    #[test]
    fn checks_accept_inflections_and_flag_forbidden_variants() {
        let glossary = Glossary {
            entries: vec![GlossaryEntry {
                forbidden: vec!["Этер".to_owned()],
                ..entry("Aether", "Эфир")
            }],
            diagnostics: Vec::new(),
        };
        assert!(
            glossary
                .check("The aether flows", "Эфиром полон мир")
                .is_empty()
        );
        let findings = glossary.check("The aether flows", "Этер течёт");
        assert_eq!(findings.len(), 2);
        assert!(glossary.check("Nothing here", "Этер").is_empty());
    }

    #[test]
    fn glossary_changes_replace_add_and_remove_in_canonical_form() {
        let current = "translation,term\nЭфир,Aether\nКристалл,Crystal\n";
        let changed = change_glossary(
            Some(current),
            vec![entry("aether", "Эфир (стихия)"), entry("Sage", "Мудрец")],
            &["CRYSTAL".to_owned()],
        )
        .expect("change");
        let entries = parse_glossary(changed.as_bytes()).expect("parse").entries;
        assert_eq!(
            entries,
            vec![entry("aether", "Эфир (стихия)"), entry("Sage", "Мудрец")]
        );
        assert!(change_glossary(Some("term,translation\n,x\n"), Vec::new(), &[]).is_err());
        assert!(change_glossary(None, vec![entry(" ", "x")], &[]).is_err());
        assert_eq!(
            change_glossary(None, vec![entry("A", "Б")], &[]).expect("new"),
            "term,translation,note,forbidden\nA,Б,,\n"
        );
    }

    #[test]
    fn project_files_are_read_from_the_repository_root() {
        let directory = tempfile::tempdir().expect("directory");
        assert!(ProjectGuide::load(directory.path()).glossary.is_none());
        fs::write(directory.path().join(GUIDANCE_FILE), "  Use «ёлочки».\n").expect("guidance");
        fs::write(directory.path().join(GLOSSARY_FILE), "term\n").expect("glossary");
        let guide = ProjectGuide::load(directory.path());
        assert_eq!(guide.guidance.as_deref(), Some("Use «ёлочки»."));
        assert!(guide.glossary.is_none());
        assert_eq!(guide.problems.len(), 1);
    }
}
