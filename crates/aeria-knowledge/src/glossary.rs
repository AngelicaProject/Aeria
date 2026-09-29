//! The project's terms: `aeria-knowledge/terms.csv`, Glossary Format v1
//! (`docs/formats/glossary-v1.md`).

use serde::Serialize;

/// Largest terms file read.
pub const MAX_GLOSSARY_BYTES: u64 = 8 * 1024 * 1024;
/// Most entries read.
pub const MAX_GLOSSARY_ENTRIES: usize = 100_000;

const COLUMNS: [&str; 5] = ["term", "translation", "note", "forbidden", "settled"];
const TERMS_FILE: &str = "aeria-knowledge/terms.csv";

/// One term.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub term: String,
    pub translation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Translations that must not be used for the term.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forbidden: Vec<String>,
    /// A person decided the term; agents do not change it without asking.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub settled: bool,
}

/// A row that was excluded, with the reason.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryDiagnostic {
    /// One-based line of the record in the file.
    pub line: u64,
    pub message: String,
}

/// A parsed terms file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Glossary {
    pub entries: Vec<GlossaryEntry>,
    pub diagnostics: Vec<GlossaryDiagnostic>,
}

/// A terms file that cannot be read at all.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{TERMS_FILE} cannot be used: {message}")]
pub struct GlossaryError {
    pub message: String,
}

fn glossary_error(message: impl Into<String>) -> GlossaryError {
    GlossaryError {
        message: message.into(),
    }
}

/// Whether a `settled` field marks the entry settled.
fn is_settled(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "1")
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
    let mut index = [None; COLUMNS.len()];
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
            settled: is_settled(field(4)),
        });
    }
    Ok(glossary)
}

/// Writes entries in the canonical form: the full header, one entry per
/// line, LF line endings, and quoting only where needed.
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
            if entry.settled { "yes" } else { "" },
        ]);
    }
    String::from_utf8(writer.into_inner().unwrap_or_default()).unwrap_or_default()
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Finds `needle` in `haystack` case-insensitively, as a whole word when it
/// starts and ends with word characters.
#[must_use]
pub fn contains_term(haystack: &str, needle: &str) -> bool {
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

    /// The entry of a term, ignoring case.
    #[must_use]
    pub fn find(&self, term: &str) -> Option<&GlossaryEntry> {
        let term = term.to_lowercase();
        self.entries
            .iter()
            .find(|entry| entry.term.to_lowercase() == term)
    }

    /// Forbidden variants a translation uses for terms of its source. A
    /// translation with one is wrong.
    #[must_use]
    pub fn forbidden_in(&self, source: &str, target: &str) -> Vec<String> {
        let mut findings = Vec::new();
        for entry in self.matches(source) {
            for forbidden in &entry.forbidden {
                if contains_term(target, forbidden) {
                    findings.push(format!(
                        "the terms forbid {forbidden:?} for {:?}; use {:?}",
                        entry.term, entry.translation
                    ));
                }
            }
        }
        findings
    }

    /// Terms of the source whose translation does not seem to be used. Only
    /// advice: inflected languages rarely keep the dictionary form, and a
    /// line may render a term another way on purpose.
    #[must_use]
    pub fn missing_in(&self, source: &str, target: &str) -> Vec<String> {
        self.matches(source)
            .into_iter()
            .filter(|entry| !contains_inflected(target, &entry.translation))
            .map(|entry| {
                format!(
                    "the terms translate {:?} as {:?}, which does not seem to be used",
                    entry.term, entry.translation
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str, translation: &str) -> GlossaryEntry {
        GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            ..GlossaryEntry::default()
        }
    }

    #[test]
    fn rows_are_read_and_invalid_rows_reported() {
        let csv = "\u{feff}translation,term,forbidden,settled\nЭфир,Aether,Этер; Эфиръ,yes\n,Empty,,\nКристалл,Crystal,,\nКристал,crystal,,\n\n\"Мудрец, старший\",\"Sage, elder\",,\n";
        let glossary = parse_glossary(csv.as_bytes()).expect("glossary");
        assert_eq!(glossary.entries.len(), 3);
        assert_eq!(glossary.entries[0].forbidden, vec!["Этер", "Эфиръ"]);
        assert!(glossary.entries[0].settled);
        assert!(!glossary.entries[1].settled);
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
                settled: true,
                ..entry("Aether", "Эфир")
            },
            entry("Sage, elder", "Мудрец"),
        ];
        let text = write_glossary(&entries);
        assert!(text.starts_with("term,translation,note,forbidden,settled\n"));
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
        assert!(glossary.find("ul'DAH").is_some());
    }

    #[test]
    fn checks_accept_inflections_and_find_forbidden_variants() {
        let glossary = Glossary {
            entries: vec![GlossaryEntry {
                forbidden: vec!["Этер".to_owned()],
                ..entry("Aether", "Эфир")
            }],
            diagnostics: Vec::new(),
        };
        assert!(
            glossary
                .missing_in("The aether flows", "Эфиром полон мир")
                .is_empty()
        );
        assert_eq!(
            glossary
                .forbidden_in("The aether flows", "Этер течёт")
                .len(),
            1
        );
        assert_eq!(
            glossary.missing_in("The aether flows", "Этер течёт").len(),
            1
        );
        assert!(glossary.forbidden_in("Nothing here", "Этер").is_empty());
    }
}
