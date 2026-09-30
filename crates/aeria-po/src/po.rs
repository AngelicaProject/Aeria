//! gettext PO files in Aeria's canonical form: read with every problem and
//! its line, written byte for byte the same for the same content.

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

/// A comment's text on one line.
fn comment_line(text: &str) -> String {
    text.replace('\r', "").replace('\n', "\\n")
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
    if entry.fuzzy {
        text.push_str("#, fuzzy\n");
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
            if rest.split(',').any(|flag| flag.trim() == "fuzzy") {
                target.entry.fuzzy = true;
            }
        } else if let Some(rest) = line.strip_prefix("#|") {
            let rest = rest.trim();
            if let Some(literal) = rest.strip_prefix("msgid ") {
                target.entry.previous = unquote(literal);
            } else if let (Some(more), Some(previous)) = (unquote(rest), &mut target.entry.previous)
            {
                previous.push_str(&more);
            }
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
