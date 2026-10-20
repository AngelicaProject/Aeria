//! gettext PO files in Aeria's canonical form: read with every problem and
//! its line, written byte for byte the same for the same content.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One entry: a string of the game and its translation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Entry {
    /// `#.` lines: what Aeria derives from the game.
    pub extracted: Vec<String>,
    /// `# ` lines: translator notes.
    pub notes: Vec<String>,
    /// `#, fuzzy`: a game update changed the source after the translation
    /// was written.
    pub fuzzy: bool,
    /// `#| msgid`: the source the translation was written for.
    pub previous: Option<String>,
    /// `#, aeria-term-exception: <term>` flags: terms of the glossary a
    /// person decided do not apply to this string.
    pub term_exceptions: Vec<String>,
    /// `# aeria-terms: {"<term>":"<fingerprint>",…}`: the terms of the
    /// source the translation was written with, by headword, each with the
    /// [`name_fingerprint`] of the term as it was then (see
    /// [`terms_written_with`] and [`Entry::outdated_terms`]).
    pub written_with: BTreeMap<String, String>,
    /// `#, aeria-reviewed: <fingerprint>`: a person reviewed the
    /// translation whose [`fingerprint`] this is. The mark holds only while
    /// the translation is still that text (see [`Entry::is_reviewed`]).
    pub reviewed: Option<String>,
    /// `msgctxt`: the entry's identity.
    pub context: String,
    /// `msgid`: the source text.
    pub source: String,
    /// `msgstr`: the translation; empty while there is none.
    pub translation: String,
    /// The line of `msgstr` in the file the entry was read from, from 1; 0
    /// for an entry that was not read.
    pub line: usize,
}

/// The header entry: comments above it and its fields.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Header {
    /// `# ` lines above the header entry.
    pub comments: Vec<String>,
    /// `Name: value` fields in order.
    pub fields: Vec<(String, String)>,
}

/// One PO file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoFile {
    pub header: Header,
    pub entries: Vec<Entry>,
    /// `#~` entries: strings the game no longer has, kept with their
    /// translations.
    pub obsolete: Vec<Entry>,
}

/// A line of a file that breaks the format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Problem {
    /// The line, from 1.
    pub line: usize,
    pub message: String,
}

/// A PO string literal.
#[must_use]
pub fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// The text of a PO string literal, or `None` when it is not one.
#[must_use]
pub fn unquote(literal: &str) -> Option<String> {
    let inner = literal.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        if character == '"' {
            return None;
        }
        if character != '\\' {
            text.push(character);
            continue;
        }
        match characters.next()? {
            'n' => text.push('\n'),
            'r' => text.push('\r'),
            't' => text.push('\t'),
            '\\' => text.push('\\'),
            '"' => text.push('"'),
            _ => return None,
        }
    }
    Some(text)
}

/// The flag of a term exception: `#, aeria-term-exception: the Maelstrom`.
pub const TERM_EXCEPTION_FLAG: &str = "aeria-term-exception";

/// The fingerprint of a glossary entry: the first twelve hexadecimal digits
/// of the SHA-256 of what a translation takes from the term: its
/// translation and own note. Its headword, forms, and case say where the
/// term is found, not how it is written, so a term the sheets give alike
/// keeps the fingerprint of the project's term it replaces. Its category
/// tells it from the other meanings of its name (see [`name_fingerprint`]).
#[must_use]
pub fn term_fingerprint(entry: &aeria_knowledge::GlossaryEntry) -> String {
    let mut text = String::new();
    for part in [
        entry.translation.as_str(),
        entry.note.as_deref().unwrap_or_default(),
    ] {
        text.push_str(part);
        text.push('\0');
    }
    fingerprint(&text)
}

/// The fingerprint of a name as a translation is written with it: the
/// [`term_fingerprint`] of its one meaning, or of the fingerprints and
/// categories of all its meanings in order.
#[must_use]
pub fn name_fingerprint(
    glossary: &aeria_knowledge::Glossary,
    entry: &aeria_knowledge::GlossaryEntry,
) -> String {
    let senses = glossary.senses(entry);
    if senses.len() <= 1 {
        return term_fingerprint(entry);
    }
    let joined: Vec<String> = senses
        .into_iter()
        .map(|sense| format!("{} {}", term_fingerprint(sense), sense.category))
        .collect();
    fingerprint(&joined.join("\0"))
}

/// The comment that records the terms a translation was written with:
/// `# aeria-terms: {"Moghome":"1d4e0f7a9c2b"}`.
pub const TERMS_COMMENT: &str = "aeria-terms:";

/// The record of the terms a translation of `entry` is written with as
/// `glossary` has them now: every term of its source, but for the terms a
/// person made an exception for, by headword, each name once, with its
/// [`name_fingerprint`].
#[must_use]
pub fn terms_written_with(
    glossary: &aeria_knowledge::Glossary,
    entry: &Entry,
) -> BTreeMap<String, String> {
    let mut written = BTreeMap::new();
    for term in glossary.matches_except(&entry.source, &entry.term_exceptions) {
        let known = written
            .keys()
            .any(|name: &String| name.to_lowercase() == term.term.to_lowercase());
        if !known {
            written.insert(term.term.clone(), name_fingerprint(glossary, term));
        }
    }
    written
}

/// The value of a terms comment: an object of headwords and fingerprints of
/// twelve hexadecimal digits; `None` for any other text, which is a note.
fn parse_terms(value: &str) -> Option<BTreeMap<String, String>> {
    let record: BTreeMap<String, String> = serde_json::from_str(value).ok()?;
    record
        .values()
        .all(|print| print.len() == 12 && print.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(record)
}

/// The flag of a reviewed translation: `#, aeria-reviewed: 3f9a1c0b7d2e`.
pub const REVIEWED_FLAG: &str = "aeria-reviewed";

/// The fingerprint of a translation in its review mark: the first twelve
/// hexadecimal digits of the SHA-256 of its text.
#[must_use]
pub fn fingerprint(translation: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(translation.as_bytes());
    digest[..6]
        .iter()
        .fold(String::with_capacity(12), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

impl Entry {
    /// A person reviewed the translation as it is: the mark's fingerprint is
    /// the translation's. A mark merged or edited onto another text holds
    /// no longer, so a review never passes for a text nobody reviewed.
    #[must_use]
    pub fn is_reviewed(&self) -> bool {
        !self.translation.is_empty()
            && self.reviewed.as_deref() == Some(fingerprint(&self.translation).as_str())
    }

    /// The entry has a review mark of another text: the translation changed
    /// after the review, by a merge or by hand.
    #[must_use]
    pub fn review_is_stale(&self) -> bool {
        self.reviewed.is_some() && !self.is_reviewed()
    }

    /// The terms of the source, but for those a person made an exception
    /// for, that the translation was not written with as `glossary` has
    /// them now: the record names another version of the term, or none. An
    /// untranslated entry has none.
    #[must_use]
    pub fn outdated_terms<'a>(
        &self,
        glossary: &'a aeria_knowledge::Glossary,
    ) -> Vec<&'a aeria_knowledge::GlossaryEntry> {
        if self.translation.is_empty() {
            return Vec::new();
        }
        let mut outdated: Vec<&aeria_knowledge::GlossaryEntry> = Vec::new();
        for term in glossary.matches_except(&self.source, &self.term_exceptions) {
            if outdated.iter().any(|known| known.same_name(term)) {
                continue;
            }
            let print = name_fingerprint(glossary, term);
            let current = self
                .written_with
                .iter()
                .any(|(name, known)| *known == print && term.is_named(name));
            if !current {
                outdated.push(term);
            }
        }
        outdated
    }

    /// Records that the translation is written with the terms of `glossary`
    /// as they are now; an untranslated entry keeps no record.
    pub fn record_terms(&mut self, glossary: &aeria_knowledge::Glossary) {
        self.written_with = if self.translation.is_empty() {
            BTreeMap::new()
        } else {
            terms_written_with(glossary, self)
        };
    }

    /// Marks the translation as reviewed by a person, or removes the mark.
    /// An untranslated entry has nothing to review and keeps no mark.
    pub fn set_reviewed(&mut self, reviewed: bool) {
        self.reviewed =
            (reviewed && !self.translation.is_empty()).then(|| fingerprint(&self.translation));
    }
}

/// Whether a term can be written in a term exception flag: flags are
/// separated by commas and end with the line.
#[must_use]
pub fn can_be_exception(term: &str) -> bool {
    let term = term.trim();
    !term.is_empty() && !term.contains([',', '\n', '\r'])
}

/// A comment as one line. Reading drops the white space a line ends with,
/// so it is not written: a file read and written again keeps its bytes. A
/// line break is text, written as `\n`, and stays.
fn comment_line(text: &str) -> String {
    text.replace('\r', "")
        .replace('\n', "\\n")
        .trim_end()
        .to_owned()
}

impl PoFile {
    /// The value of a header field.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.header
            .fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    /// The file's text in canonical form.
    #[must_use]
    pub fn write(&self) -> String {
        let mut text = String::new();
        for comment in &self.header.comments {
            let _ = writeln!(text, "# {}", comment_line(comment));
        }
        text.push_str("msgid \"\"\nmsgstr \"\"\n");
        for (name, value) in &self.header.fields {
            let _ = writeln!(text, "{}", quote(&format!("{name}: {value}\n")));
        }
        for entry in &self.entries {
            text.push('\n');
            write_entry(&mut text, entry, "");
        }
        for entry in &self.obsolete {
            text.push('\n');
            write_entry(&mut text, entry, "#~ ");
        }
        text
    }

    /// Reads a file. Every line that breaks the format is a problem; the
    /// entry it belongs to is left out and the others are read.
    #[must_use]
    pub fn parse(text: &str) -> (Self, Vec<Problem>) {
        Parser::default().parse(text)
    }
}

fn write_entry(text: &mut String, entry: &Entry, prefix: &str) {
    for line in &entry.extracted {
        let _ = writeln!(text, "#. {}", comment_line(line));
    }
    for note in &entry.notes {
        if note.is_empty() {
            text.push_str("#\n");
        } else {
            let _ = writeln!(text, "# {}", comment_line(note));
        }
    }
    if !entry.written_with.is_empty()
        && let Ok(record) = serde_json::to_string(&entry.written_with)
    {
        let _ = writeln!(text, "# {TERMS_COMMENT} {record}");
    }
    let mut flags: Vec<String> = Vec::new();
    if entry.fuzzy {
        flags.push("fuzzy".to_owned());
    }
    flags.extend(
        entry
            .term_exceptions
            .iter()
            .map(|term| format!("{TERM_EXCEPTION_FLAG}: {term}")),
    );
    if let Some(reviewed) = &entry.reviewed {
        flags.push(format!("{REVIEWED_FLAG}: {reviewed}"));
    }
    if !flags.is_empty() {
        let _ = writeln!(text, "#, {}", flags.join(", "));
    }
    if let Some(previous) = &entry.previous {
        let _ = writeln!(text, "#| msgid {}", quote(previous));
    }
    let _ = writeln!(text, "{prefix}msgctxt {}", quote(&entry.context));
    let _ = writeln!(text, "{prefix}msgid {}", quote(&entry.source));
    let _ = writeln!(text, "{prefix}msgstr {}", quote(&entry.translation));
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Field {
    #[default]
    None,
    Context,
    Id,
    Str,
    /// The entry broke the format; its lines up to the next entry are
    /// skipped.
    Broken,
}

/// Fields of an entry already read, as bits.
const CONTEXT: u8 = 1;
const ID: u8 = 2;
const STR: u8 = 4;

/// An entry being read.
#[derive(Default)]
struct Open {
    entry: Entry,
    /// [`CONTEXT`], [`ID`], and [`STR`] as they are read.
    read: u8,
    obsolete: bool,
}

impl Open {
    const fn has(&self, field: u8) -> bool {
        self.read & field != 0
    }
}

#[derive(Default)]
struct Parser {
    file: PoFile,
    problems: Vec<Problem>,
    open: Option<Open>,
    field: Field,
    /// Comments read for the next entry.
    comments: Open,
    header_done: bool,
}

impl Parser {
    fn parse(mut self, text: &str) -> (PoFile, Vec<Problem>) {
        for (index, raw) in text.lines().enumerate() {
            self.line(index + 1, raw.trim_end_matches('\r'));
        }
        self.finish();
        (self.file, self.problems)
    }

    fn problem(&mut self, line: usize, message: &str) {
        self.problems.push(Problem {
            line,
            message: message.to_owned(),
        });
        self.open = None;
        self.comments = Open::default();
        self.field = Field::Broken;
    }

    /// Ends the entry being read.
    fn finish(&mut self) {
        let Some(open) = self.open.take() else {
            return;
        };
        self.field = Field::None;
        if !open.has(CONTEXT) && open.entry.source.is_empty() && !self.header_done {
            self.header_done = true;
            self.file.header.comments = open.entry.notes;
            for line in open.entry.translation.split('\n') {
                if let Some((name, value)) = line.split_once(": ") {
                    self.file
                        .header
                        .fields
                        .push((name.to_owned(), value.to_owned()));
                }
            }
            return;
        }
        self.header_done = true;
        if open.has(STR) {
            if open.obsolete {
                self.file.obsolete.push(open.entry);
            } else {
                self.file.entries.push(open.entry);
            }
        }
    }

    /// Starts an entry with the comments read for it.
    fn start(&mut self, obsolete: bool) {
        self.finish();
        let mut open = std::mem::take(&mut self.comments);
        open.obsolete = obsolete;
        self.open = Some(open);
    }

    fn line(&mut self, number: usize, line: &str) {
        if ["<<<<<<<", "=======", ">>>>>>>", "|||||||"]
            .iter()
            .any(|marker| line.starts_with(marker))
        {
            self.problem(
                number,
                "a Git conflict marker: keep one side of the conflict and remove the markers",
            );
            return;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if matches!(self.field, Field::Str | Field::Broken) {
                self.finish();
                self.field = Field::None;
            }
            return;
        }
        if let Some(rest) = trimmed.strip_prefix("#~") {
            self.field_line(number, rest.trim_start(), true);
            return;
        }
        if trimmed.starts_with('#') {
            if self.field == Field::Str {
                self.finish();
            }
            if self.field == Field::Broken {
                self.field = Field::None;
            }
            self.comment(trimmed);
            return;
        }
        self.field_line(number, trimmed, false);
    }

    fn comment(&mut self, line: &str) {
        let target = &mut self.comments;
        if let Some(rest) = line.strip_prefix("#.") {
            target.entry.extracted.push(unescape_comment(rest));
        } else if let Some(rest) = line.strip_prefix("#,") {
            for flag in rest.split(',').map(str::trim) {
                if flag == "fuzzy" {
                    target.entry.fuzzy = true;
                } else if let Some(term) = flag
                    .strip_prefix(TERM_EXCEPTION_FLAG)
                    .and_then(|rest| rest.strip_prefix(':'))
                    .map(str::trim)
                    .filter(|term| !term.is_empty())
                    && !target
                        .entry
                        .term_exceptions
                        .iter()
                        .any(|known| known == term)
                {
                    target.entry.term_exceptions.push(term.to_owned());
                } else if let Some(mark) = flag
                    .strip_prefix(REVIEWED_FLAG)
                    .and_then(|rest| rest.strip_prefix(':'))
                    .map(str::trim)
                    .filter(|mark| !mark.is_empty())
                {
                    target.entry.reviewed = Some(mark.to_owned());
                }
            }
        } else if let Some(rest) = line.strip_prefix("#|") {
            let rest = rest.trim();
            if let Some(literal) = rest.strip_prefix("msgid ") {
                target.entry.previous = unquote(literal);
            } else if let (Some(more), Some(previous)) = (unquote(rest), &mut target.entry.previous)
            {
                previous.push_str(&more);
            }
        } else if let Some(record) = line[1..]
            .trim_start()
            .strip_prefix(TERMS_COMMENT)
            .and_then(|value| parse_terms(value.trim()))
        {
            // Read from the line as written: an escape in a headword is the
            // record's, not a line break of a note.
            target.entry.written_with.extend(record);
        } else {
            target.entry.notes.push(unescape_comment(&line[1..]));
        }
    }

    fn field_line(&mut self, number: usize, line: &str, obsolete: bool) {
        if self.field == Field::Broken
            && !line.starts_with("msgctxt ")
            && !line.starts_with("msgid ")
        {
            return;
        }
        let result: Result<(), &str> = if let Some(rest) = line.strip_prefix("msgctxt ") {
            self.start(obsolete);
            self.field = Field::Context;
            match (unquote(rest), &mut self.open) {
                (Some(context), Some(open)) => {
                    open.entry.context = context;
                    open.read |= CONTEXT;
                    Ok(())
                }
                _ => Err("msgctxt is not a quoted string"),
            }
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            let continues = self
                .open
                .as_ref()
                .is_some_and(|open| open.has(CONTEXT) && !open.has(ID));
            if !continues {
                self.start(obsolete);
            }
            self.field = Field::Id;
            match (unquote(rest), &mut self.open) {
                (Some(source), Some(open)) => {
                    open.entry.source = source;
                    open.read |= ID;
                    Ok(())
                }
                _ => Err("msgid is not a quoted string"),
            }
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            self.field = Field::Str;
            match (unquote(rest), &mut self.open) {
                (Some(translation), Some(open)) if open.has(ID) => {
                    open.entry.translation = translation;
                    open.entry.line = number;
                    open.read |= STR;
                    Ok(())
                }
                (None, _) => Err(
                    "msgstr is not a quoted string: write it as msgstr \"…\", with \\\" for a quote and \\\\ for a backslash",
                ),
                _ => Err("msgstr without msgid"),
            }
        } else if line.starts_with('"') {
            match (unquote(line), self.field, &mut self.open) {
                (None, _, _) => {
                    Err("not a quoted string: write \\\" for a quote and \\\\ for a backslash")
                }
                (Some(more), Field::Context, Some(open)) => {
                    open.entry.context.push_str(&more);
                    Ok(())
                }
                (Some(more), Field::Id, Some(open)) => {
                    open.entry.source.push_str(&more);
                    Ok(())
                }
                (Some(more), Field::Str, Some(open)) => {
                    open.entry.translation.push_str(&more);
                    Ok(())
                }
                _ => Err("a string outside an entry"),
            }
        } else {
            Err(
                "not a PO line: comments start with #, and the fields are msgctxt, msgid, and msgstr",
            )
        };
        if let Err(message) = result {
            self.problem(number, message);
        }
    }
}

/// A comment's text: after the marker and one space.
fn unescape_comment(rest: &str) -> String {
    rest.strip_prefix(' ').unwrap_or(rest).replace("\\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PoFile {
        PoFile {
            header: Header {
                comments: vec!["quest/000/A — «Title»".to_owned()],
                fields: vec![
                    ("Language".to_owned(), "ru".to_owned()),
                    ("X-Game-Version".to_owned(), "1".to_owned()),
                ],
            },
            entries: vec![
                Entry {
                    extracted: vec!["ja: 名前".to_owned(), "speaker: A".to_owned()],
                    notes: vec!["a note".to_owned()],
                    fuzzy: true,
                    previous: Some("Old \"name\"".to_owned()),
                    term_exceptions: vec!["the Maelstrom".to_owned(), "Scions".to_owned()],
                    written_with: BTreeMap::from([
                        ("Mount Gulg".to_owned(), "3f9a1c0b7d2e".to_owned()),
                        ("Moghome \"\\n\"".to_owned(), "1d4e0f7a9c2b".to_owned()),
                    ]),
                    reviewed: Some(fingerprint("Имя \\<")),
                    context: "quest/000/A:TEXT_A_SEQ_00:1".to_owned(),
                    source: "Name <if $gn4>a<else>b</if>".to_owned(),
                    translation: "Имя \\<".to_owned(),
                    line: 0,
                },
                Entry {
                    context: "quest/000/A:TEXT_A_SEQ_01:1".to_owned(),
                    source: "Next".to_owned(),
                    ..Entry::default()
                },
            ],
            obsolete: vec![Entry {
                notes: vec!["kept".to_owned()],
                context: "quest/000/A:TEXT_A_OLD:1".to_owned(),
                source: "Gone".to_owned(),
                translation: "Ушло".to_owned(),
                ..Entry::default()
            }],
        }
    }

    fn without_lines(mut file: PoFile) -> PoFile {
        for entry in file.entries.iter_mut().chain(file.obsolete.iter_mut()) {
            entry.line = 0;
        }
        file
    }

    #[test]
    fn a_written_file_reads_back_the_same() {
        let text = sample().write();
        let (read, problems) = PoFile::parse(&text);
        assert!(problems.is_empty(), "{problems:?}\n{text}");
        assert_eq!(without_lines(read.clone()), sample());
        assert_eq!(read.write(), text);
        assert_eq!(read.field("Language"), Some("ru"));
        assert!(text.contains(&format!(
            "# a note\n# aeria-terms: {{\"Moghome \\\"\\\\n\\\"\":\"1d4e0f7a9c2b\",\"Mount Gulg\":\"3f9a1c0b7d2e\"}}\n#, fuzzy, aeria-term-exception: the Maelstrom, aeria-term-exception: Scions, aeria-reviewed: {}\n",
            fingerprint("Имя \\<")
        )));
    }

    #[test]
    fn a_comment_ending_in_white_space_is_written_as_it_reads_back() {
        let mut file = sample();
        file.entries[0].extracted = vec!["de: bis ".to_owned(), "column 4:  ".to_owned()];
        file.entries[0].notes = vec!["a note ".to_owned()];
        let text = file.write();
        assert!(
            text.contains("#. de: bis\n#. column 4:\n# a note\n"),
            "{text}"
        );
        let (read, problems) = PoFile::parse(&text);
        assert!(problems.is_empty(), "{problems:?}\n{text}");
        assert_eq!(read.write(), text);
    }

    #[test]
    fn a_comment_ending_in_a_line_break_keeps_it() {
        let mut file = sample();
        file.entries[0].extracted = vec!["column 14: sung.\n".to_owned(), "fr: IL.\r\n".to_owned()];
        let text = file.write();
        assert!(
            text.contains("#. column 14: sung.\\n\n#. fr: IL.\\n\n"),
            "{text}"
        );
        let (read, problems) = PoFile::parse(&text);
        assert!(problems.is_empty(), "{problems:?}\n{text}");
        assert_eq!(
            read.entries[0].extracted,
            ["column 14: sung.\n", "fr: IL.\n"]
        );
        assert_eq!(read.write(), text);
    }

    #[test]
    fn a_review_holds_only_for_the_text_reviewed() {
        assert_eq!(fingerprint("Прогулка в парке").len(), 12);
        assert_eq!(
            fingerprint("Прогулка в парке"),
            fingerprint("Прогулка в парке")
        );
        assert_ne!(
            fingerprint("Прогулка в парке"),
            fingerprint("Прогулка по парку")
        );
        let mut entry = Entry {
            translation: "Прогулка в парке".to_owned(),
            ..Entry::default()
        };
        assert!(!entry.is_reviewed() && !entry.review_is_stale());
        entry.set_reviewed(true);
        assert!(entry.is_reviewed());
        // A merge or a hand edit brings another text under the same mark.
        "Прогулка по парку".clone_into(&mut entry.translation);
        assert!(!entry.is_reviewed());
        assert!(entry.review_is_stale());
        entry.set_reviewed(false);
        assert_eq!(entry.reviewed, None);
        entry.translation.clear();
        entry.set_reviewed(true);
        assert_eq!(entry.reviewed, None, "nothing to review");
        let text =
            "#, aeria-reviewed:  abc , fuzzy\nmsgctxt \"S:1:0:0\"\nmsgid \"x\"\nmsgstr \"y\"\n";
        let read = PoFile::parse(text).0.entries[0].clone();
        assert_eq!(read.reviewed.as_deref(), Some("abc"));
        assert!(read.fuzzy && read.review_is_stale());
    }

    #[test]
    fn term_exceptions_are_read_without_fuzzy_and_other_flags_are_left() {
        let text = "msgctxt \"S:1:0:0\"\nmsgid \"x\"\nmsgstr \"\"\n";
        let with = |flags: &str| PoFile::parse(&format!("{flags}\n{text}")).0.entries[0].clone();
        let entry = with("#, aeria-term-exception:  Maelstrom , c-format");
        assert!(!entry.fuzzy);
        assert_eq!(entry.term_exceptions, ["Maelstrom"]);
        assert!(with("#, aeria-term-exception:").term_exceptions.is_empty());
        assert!(can_be_exception("the Maelstrom"));
        assert!(!can_be_exception("Sage, elder"));
        assert!(!can_be_exception(" "));
    }

    #[test]
    fn a_translation_is_current_with_the_terms_it_was_written_with() {
        let term = |term: &str, translation: &str| aeria_knowledge::GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            ..aeria_knowledge::GlossaryEntry::default()
        };
        let glossary = aeria_knowledge::Glossary::new(vec![
            term("Moghome", "Моглин"),
            term("Kupo", "Купо"),
            term("Ishgard", "Ишгард"),
        ]);
        let mut entry = Entry {
            source: "Kupo! Welcome to the Moghome.".to_owned(),
            translation: "Купо! Добро пожаловать в Моглин.".to_owned(),
            ..Entry::default()
        };
        let names = |terms: Vec<&aeria_knowledge::GlossaryEntry>| -> Vec<String> {
            terms.iter().map(|term| term.term.clone()).collect()
        };
        assert_eq!(names(entry.outdated_terms(&glossary)), ["Moghome", "Kupo"]);
        entry.record_terms(&glossary);
        assert_eq!(
            entry.written_with.keys().collect::<Vec<_>>(),
            ["Kupo", "Moghome"]
        );
        assert!(entry.outdated_terms(&glossary).is_empty());
        // Another translation, note, or meaning of a term is another version.
        let changed =
            aeria_knowledge::Glossary::new(vec![term("Moghome", "Моглодом"), term("Kupo", "Купо")]);
        assert_eq!(names(entry.outdated_terms(&changed)), ["Moghome"]);
        // A term the sheets give alike, under another headword, case, or
        // forms, is the same version: the translation takes nothing else.
        let sheets = aeria_knowledge::Glossary::new(vec![
            aeria_knowledge::GlossaryEntry {
                forms: vec!["Moghome".to_owned(), "Moghomes".to_owned()],
                match_case: true,
                ..term("the Moghome", "Моглин")
            },
            term("kupo", "Купо"),
        ]);
        assert!(entry.outdated_terms(&sheets).is_empty());
        // A term a person made an exception for is not asked for.
        entry.term_exceptions = vec!["moghome".to_owned()];
        assert!(entry.outdated_terms(&changed).is_empty());
        // The record is read back; a comment that is not one is a note.
        let text = PoFile {
            entries: vec![entry.clone()],
            ..PoFile::default()
        }
        .write();
        let read = PoFile::parse(&text).0.entries[0].clone();
        assert_eq!(read.written_with, entry.written_with);
        assert!(read.notes.is_empty());
        let note =
            "# aeria-terms: {\"Kupo\":\"xyz\"}\nmsgctxt \"S:1:0:0\"\nmsgid \"x\"\nmsgstr \"y\"\n";
        let read = PoFile::parse(note).0.entries[0].clone();
        assert!(read.written_with.is_empty());
        assert_eq!(read.notes, ["aeria-terms: {\"Kupo\":\"xyz\"}"]);
        // An untranslated entry keeps no record and has nothing outdated.
        entry.translation.clear();
        entry.record_terms(&glossary);
        assert!(entry.written_with.is_empty());
        assert!(entry.outdated_terms(&glossary).is_empty());
    }

    #[test]
    fn strings_round_trip_and_bad_literals_are_refused() {
        for text in ["plain", "a \"quote\" and \\<br>", "tab\tand\nline", ""] {
            assert_eq!(unquote(&quote(text)).as_deref(), Some(text));
        }
        assert_eq!(unquote("\"a\"b\""), None);
        assert_eq!(unquote("\"a\\q\""), None);
        assert_eq!(unquote("a"), None);
    }

    #[test]
    fn continued_strings_are_joined_and_lines_kept() {
        let text = "msgid \"\"\nmsgstr \"\"\n\"Language: ru\\n\"\n\nmsgctxt \"S:1:0:0\"\nmsgid \"Lo\"\n\"ng\"\nmsgstr \"\"\n\"Длин\"\n\"ное\"\n";
        let (file, problems) = PoFile::parse(text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].source, "Long");
        assert_eq!(file.entries[0].translation, "Длинное");
        assert_eq!(file.entries[0].line, 8);
    }

    #[test]
    fn a_broken_entry_is_a_problem_and_the_others_are_read() {
        let text = "msgid \"\"\nmsgstr \"\"\n\nmsgctxt \"S:1:0:0\"\nmsgid \"x\"\nmsgstr Имя\n\nmsgctxt \"S:2:0:0\"\nmsgid \"y\"\nmsgstr \"Да\"\n";
        let (file, problems) = PoFile::parse(text);
        assert_eq!(
            problems
                .iter()
                .map(|problem| problem.line)
                .collect::<Vec<_>>(),
            [6]
        );
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].translation, "Да");
    }

    #[test]
    fn conflict_markers_are_problems() {
        let text = "msgid \"\"\nmsgstr \"\"\n\nmsgctxt \"S:1:0:0\"\nmsgid \"x\"\n<<<<<<< HEAD\nmsgstr \"А\"\n=======\nmsgstr \"Б\"\n>>>>>>> other\n\nmsgctxt \"S:2:0:0\"\nmsgid \"y\"\nmsgstr \"Да\"\n";
        let (file, problems) = PoFile::parse(text);
        assert_eq!(
            problems
                .iter()
                .map(|problem| problem.line)
                .collect::<Vec<_>>(),
            [6, 8, 10]
        );
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].context, "S:2:0:0");
    }
}
