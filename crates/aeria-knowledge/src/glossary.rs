//! The project's terms: `aeria-knowledge/terms.csv`, Glossary Format v1
//! (`docs/formats/glossary-v1.md`).

use serde::Serialize;

/// Largest terms file read. The entries are not counted: a project that
/// keeps every name it agrees on as a term has tens of thousands.
pub const MAX_GLOSSARY_BYTES: u64 = 64 * 1024 * 1024;

const COLUMNS: [&str; 6] = ["term", "translation", "forms", "note", "case", "category"];
const TERMS_FILE: &str = "aeria-knowledge/terms.csv";

/// One term.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    /// The term's headword, which names it: in term exceptions, and where
    /// it is shown.
    pub term: String,
    pub translation: String,
    /// Other forms of the term in the source, matched as the term:
    /// `linkshells` beside `linkshell`, `the Scions` beside `Scions of the
    /// Seventh Dawn`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The note of the term's category, which the project's sheets give a
    /// term of `terms.csv`: said before [`Self::note`] wherever the term is
    /// explained, but not part of the term as written or decided for.
    #[serde(skip)]
    pub category_note: Option<String>,
    /// The term matches only with its case as written, but for a capital
    /// first letter: `the Maelstrom` matches `The Maelstrom`, not
    /// `the maelstrom`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub match_case: bool,
    /// What kind of thing the term names in this meaning (`место`,
    /// `существо`): a name may have one meaning per category.
    pub category: String,
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
#[derive(Clone, Debug, Default)]
pub struct Glossary {
    pub entries: Vec<GlossaryEntry>,
    pub diagnostics: Vec<GlossaryDiagnostic>,
    /// Finds the terms of `entries` in a text in one pass, made on first use.
    finder: std::sync::OnceLock<Finder>,
    /// The entries by their lowercase names, made on first use.
    names: std::sync::OnceLock<Names>,
}

impl GlossaryEntry {
    /// What is said of the term: the note of its category, then its own
    /// note, each line once. `None` when neither says anything.
    #[must_use]
    pub fn full_note(&self) -> Option<String> {
        let mut lines: Vec<&str> = Vec::new();
        for line in [&self.category_note, &self.note]
            .into_iter()
            .flatten()
            .flat_map(|note| note.lines())
        {
            let line = line.trim();
            if !line.is_empty() && !lines.contains(&line) {
                lines.push(line);
            }
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// The forms of the term in the source: its headword, then its other
    /// forms.
    pub fn all_forms(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.term.as_str()).chain(self.forms.iter().map(String::as_str))
    }

    /// Whether this entry and `other` are meanings of one name: the same
    /// headword, ignoring case.
    #[must_use]
    pub fn same_name(&self, other: &Self) -> bool {
        self.term.to_lowercase() == other.term.to_lowercase()
    }

    /// Whether the strings of `exceptions`, a string's term exceptions, name
    /// the term by any of its forms, ignoring case.
    #[must_use]
    pub fn is_excepted(&self, exceptions: &[String]) -> bool {
        exceptions.iter().any(|exception| self.is_named(exception))
    }

    /// Whether `name` is one of the term's forms, its headword included,
    /// ignoring case.
    #[must_use]
    pub fn is_named(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.all_forms().any(|form| form.to_lowercase() == name)
    }

    /// Whether this meaning and `other` have one category, ignoring case.
    #[must_use]
    pub fn same_category(&self, other: &Self) -> bool {
        self.category.to_lowercase() == other.category.to_lowercase()
    }
}

impl PartialEq for Glossary {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries && self.diagnostics == other.diagnostics
    }
}

impl Eq for Glossary {}

/// The lowercase forms of a glossary's terms in one automaton, which finds
/// in a lowercase text every term that may occur, overlapping ones too. Most
/// strings have no term, and a string's terms are then matched one by one.
#[derive(Clone, Debug)]
struct Finder {
    automaton: Option<aho_corasick::AhoCorasick>,
    /// The entry of each pattern of the automaton.
    owners: Vec<usize>,
    /// The entries the automaton was made for: entries added since are
    /// matched one by one.
    entries: usize,
}

/// The entries of a glossary by their lowercase forms and headwords, so a
/// term is found by its name without lowercasing every entry.
#[derive(Clone, Debug)]
struct Names {
    /// The first entry with each form.
    forms: std::collections::HashMap<String, usize>,
    /// Every entry with each headword, in order.
    headwords: std::collections::HashMap<String, Vec<usize>>,
    /// The entries the index was made for: with others, entries are
    /// compared one by one.
    entries: usize,
}

impl Glossary {
    /// A glossary of `entries`.
    #[must_use]
    pub fn new(entries: Vec<GlossaryEntry>) -> Self {
        Self {
            entries,
            ..Self::default()
        }
    }

    /// Adds `entries` after the glossary's own: the terms of the declared
    /// sheets, which come after those of `terms.csv`.
    pub fn append(&mut self, entries: Vec<GlossaryEntry>) {
        self.entries.extend(entries);
        self.finder = std::sync::OnceLock::new();
        self.names = std::sync::OnceLock::new();
    }

    /// The meanings of the name of `entry`: every entry with its headword,
    /// ignoring case, in order.
    #[must_use]
    pub fn senses(&self, entry: &GlossaryEntry) -> Vec<&GlossaryEntry> {
        let Some(names) = self.names() else {
            return self
                .entries
                .iter()
                .filter(|other| other.same_name(entry))
                .collect();
        };
        names
            .headwords
            .get(&entry.term.to_lowercase())
            .into_iter()
            .flatten()
            .filter_map(|&index| self.entries.get(index))
            .collect()
    }

    /// The index of the entries by name, unless entries were added since it
    /// was made.
    fn names(&self) -> Option<&Names> {
        let names = self.names.get_or_init(|| {
            let mut forms = std::collections::HashMap::new();
            let mut headwords: std::collections::HashMap<String, Vec<usize>> =
                std::collections::HashMap::new();
            for (index, entry) in self.entries.iter().enumerate() {
                for form in entry.all_forms() {
                    forms.entry(form.to_lowercase()).or_insert(index);
                }
                headwords
                    .entry(entry.term.to_lowercase())
                    .or_default()
                    .push(index);
            }
            Names {
                forms,
                headwords,
                entries: self.entries.len(),
            }
        });
        (names.entries == self.entries.len()).then_some(names)
    }

    /// Indexes of the entries whose term may occur in `lower`, the lowercase
    /// text, in order.
    fn candidates(&self, lower: &str) -> Vec<usize> {
        let finder = self.finder.get_or_init(|| {
            let (owners, patterns): (Vec<usize>, Vec<String>) = self
                .entries
                .iter()
                .enumerate()
                .flat_map(|(index, entry)| {
                    matched_forms(entry).map(move |form| (index, form.to_lowercase()))
                })
                .unzip();
            Finder {
                automaton: aho_corasick::AhoCorasick::builder()
                    .match_kind(aho_corasick::MatchKind::Standard)
                    .build(patterns)
                    .ok(),
                owners,
                entries: self.entries.len(),
            }
        });
        let Some(automaton) = finder
            .automaton
            .as_ref()
            .filter(|_| finder.entries == self.entries.len())
        else {
            return (0..self.entries.len()).collect();
        };
        let mut found: Vec<usize> = automaton
            .find_overlapping_iter(lower)
            .map(|found| finder.owners[found.pattern().as_usize()])
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }
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

/// Whether a `case` field is set.
fn is_yes(value: &str) -> bool {
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
    let index = header_index(&headers)?;
    let mut glossary = Glossary::default();
    // Each lowercase form with the entries that have it.
    let mut seen: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
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
        let category = field(5);
        if term.is_empty() || translation.is_empty() || category.is_empty() {
            reject("the term, its translation, and its category must not be empty".to_owned());
            continue;
        }
        let forms: Vec<String> = field(2)
            .split(';')
            .map(str::trim)
            .filter(|form| !form.is_empty())
            .map(str::to_owned)
            .collect();
        // A name repeats as another meaning only with another category; a
        // form is shared only by meanings of one name.
        let candidate = GlossaryEntry {
            term: term.to_owned(),
            category: category.to_owned(),
            ..GlossaryEntry::default()
        };
        let mut own = std::collections::HashSet::new();
        if let Some(repeated) = std::iter::once(term)
            .chain(forms.iter().map(String::as_str))
            .find(|form| {
                let lower = form.to_lowercase();
                let taken = seen.get(&lower).is_some_and(|holders| {
                    holders.iter().any(|earlier| {
                        let earlier = &glossary.entries[*earlier];
                        !earlier.same_name(&candidate) || earlier.same_category(&candidate)
                    })
                });
                taken || !own.insert(lower)
            })
        {
            reject(format!("the term {repeated:?} is already defined above"));
            continue;
        }
        let at = glossary.entries.len();
        for form in own {
            seen.entry(form).or_default().push(at);
        }
        glossary.entries.push(GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            forms,
            note: Some(field(3))
                .filter(|note| !note.is_empty())
                .map(str::to_owned),
            category_note: None,
            match_case: is_yes(field(4)),
            category: category.to_owned(),
        });
    }
    Ok(glossary)
}

/// The position of each known column in the header: an unknown or repeated
/// column, or a missing `term`, `translation`, or `category`, refuses the
/// file.
fn header_index(
    headers: &csv::StringRecord,
) -> Result<[Option<usize>; COLUMNS.len()], GlossaryError> {
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
    if index[0].is_none() || index[1].is_none() || index[5].is_none() {
        return Err(glossary_error(
            "the header must name the columns term, translation, and category",
        ));
    }
    Ok(index)
}

/// Writes entries in the canonical form: the header (`case` only when an
/// entry matches case), one entry per line, LF line endings, and quoting
/// only where needed.
#[must_use]
pub fn write_glossary(entries: &[GlossaryEntry]) -> String {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .flexible(true)
        .from_writer(Vec::new());
    // `case` is written only when an entry matches case, so a file without
    // it stays as it was.
    let with_case = entries.iter().any(|entry| entry.match_case);
    let wanted = |column: &str| column != "case" || with_case;
    let header: Vec<&str> = COLUMNS
        .iter()
        .copied()
        .filter(|column| wanted(column))
        .collect();
    let _ = writer.write_record(&header);
    for entry in entries {
        let forms = entry.forms.join("; ");
        let record = [
            entry.term.as_str(),
            entry.translation.as_str(),
            forms.as_str(),
            entry.note.as_deref().unwrap_or_default(),
            if entry.match_case { "yes" } else { "" },
            entry.category.as_str(),
        ];
        let fields: Vec<&str> = COLUMNS
            .iter()
            .zip(record)
            .filter(|(column, _)| wanted(column))
            .map(|(_, value)| value)
            .collect();
        let _ = writer.write_record(&fields);
    }
    String::from_utf8(writer.into_inner().unwrap_or_default()).unwrap_or_default()
}

/// The text of macro text, where terms are looked for: each tag, with its
/// arguments, becomes a space, so a sheet name such as `Aetheryte` in
/// `<sheet Aetheryte $n1 8>` is not a word of the string, while text between
/// tags, such as the branches of a condition, is. A `>` inside quotes or
/// parentheses (`<if ($n1 > 0)>`) does not end a tag; `\<` is a literal `<`.
#[must_use]
pub fn text_of(macro_text: &str) -> std::borrow::Cow<'_, str> {
    text_with(macro_text, ' ')
}

/// The text of macro text as [`text_of`] reads it, with `fill` in place of
/// each tag.
fn text_with(macro_text: &str, fill: char) -> std::borrow::Cow<'_, str> {
    if !macro_text.contains('<') {
        return std::borrow::Cow::Borrowed(macro_text);
    }
    let mut text = String::with_capacity(macro_text.len());
    let mut in_tag = false;
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for character in macro_text.chars() {
        if escaped {
            escaped = false;
            if !in_tag {
                text.push(character);
            }
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if !in_tag {
            if character == '<' {
                in_tag = true;
                depth = 0;
                quoted = false;
                text.push(fill);
            } else {
                text.push(character);
            }
            continue;
        }
        match character {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            '>' if !quoted && depth == 0 => in_tag = false,
            _ => {}
        }
    }
    std::borrow::Cow::Owned(text)
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Finds `needle` in `haystack` case-insensitively, as a whole word when it
/// starts and ends with word characters.
#[must_use]
pub fn contains_term(haystack: &str, needle: &str) -> bool {
    !Folded::new(haystack).spans(needle, false).is_empty()
}

/// A text with its lowercase form, and for each byte of the lowercase form
/// the byte of the text it comes from, so spans found in either are spans
/// of the text.
struct Folded<'a> {
    text: &'a str,
    lower: String,
    origin: Vec<usize>,
}

impl<'a> Folded<'a> {
    fn new(text: &'a str) -> Self {
        let mut lower = String::with_capacity(text.len());
        let mut origin = Vec::with_capacity(text.len() + 1);
        for (index, character) in text.char_indices() {
            for folded in character.to_lowercase() {
                lower.push(folded);
                origin.resize(lower.len(), index);
            }
        }
        origin.push(text.len());
        Self {
            text,
            lower,
            origin,
        }
    }

    /// The byte spans of the text where `needle` occurs as a whole word
    /// (when it starts and ends with word characters): ignoring case, or
    /// with its case as written but for a capital first letter.
    fn spans(&self, needle: &str, match_case: bool) -> Vec<std::ops::Range<usize>> {
        let mut found = Vec::new();
        if needle.is_empty() {
            return found;
        }
        let starts_word = needle.chars().next().is_some_and(is_word);
        let ends_word = needle.chars().last().is_some_and(is_word);
        let whole = |haystack: &str, start: usize, end: usize| {
            let before = haystack[..start].chars().next_back();
            let after = haystack[end..].chars().next();
            (!starts_word || !before.is_some_and(is_word))
                && (!ends_word || !after.is_some_and(is_word))
        };
        if match_case {
            let mut capital = String::new();
            let mut characters = needle.chars();
            if let Some(first) = characters.next() {
                capital.extend(first.to_uppercase());
                capital.push_str(characters.as_str());
            }
            for form in [needle, capital.as_str()] {
                for (start, _) in self.text.match_indices(form) {
                    let end = start + form.len();
                    if whole(self.text, start, end) && !found.contains(&(start..end)) {
                        found.push(start..end);
                    }
                }
                if capital == needle {
                    break;
                }
            }
            found.sort_by_key(|span| span.start);
        } else {
            let needle = needle.to_lowercase();
            for (start, _) in self.lower.match_indices(&needle) {
                let end = start + needle.len();
                if whole(&self.lower, start, end) {
                    found.push(self.origin[start]..self.origin[end]);
                }
            }
        }
        found
    }
}

/// The English plural of a form's last word: with `s`, with `es` after `s`,
/// `x`, `z`, `ch`, or `sh`, and with `ies` for a `y` after a consonant
/// (`gladiators`, `ahrimans`, `fairies`). None for a form that does not end
/// with a Latin letter.
fn plural(form: &str) -> Option<String> {
    let last = form.chars().last().filter(char::is_ascii_alphabetic)?;
    let lower = form.to_ascii_lowercase();
    if last.eq_ignore_ascii_case(&'y') {
        let before = lower.chars().rev().nth(1);
        if before.is_some_and(|letter| letter.is_ascii_alphabetic() && !"aeiou".contains(letter)) {
            return Some(format!("{}ies", &form[..form.len() - 1]));
        }
    }
    if ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|end| lower.ends_with(end))
    {
        return Some(format!("{form}es"));
    }
    Some(format!("{form}s"))
}

/// The forms a term matches: each of its forms and that form's plural.
fn matched_forms(entry: &GlossaryEntry) -> impl Iterator<Item = String> + '_ {
    entry
        .all_forms()
        .flat_map(|form| std::iter::once(form.to_owned()).chain(plural(form)))
}

/// The byte spans of `folded` where a form of `entry` occurs, in order.
fn entry_spans(folded: &Folded<'_>, entry: &GlossaryEntry) -> Vec<std::ops::Range<usize>> {
    let mut spans: Vec<std::ops::Range<usize>> = matched_forms(entry)
        .flat_map(|form| folded.spans(&form, entry.match_case))
        .collect();
    spans.sort_by_key(|span| (span.start, span.end));
    spans.dedup();
    spans
}

/// Whether `span` lies inside a longer span of `spans`.
fn covered(span: &std::ops::Range<usize>, spans: &[std::ops::Range<usize>]) -> bool {
    spans.iter().any(|longer| {
        longer.len() > span.len() && longer.start <= span.start && span.end <= longer.end
    })
}

/// How many occurrences sorted `spans` are: spans that overlap, such as
/// `the Scions` and `Scions of the Seventh Dawn` in `the Scions of the
/// Seventh Dawn`, are one.
fn count_spans<'a>(spans: impl IntoIterator<Item = &'a std::ops::Range<usize>>) -> usize {
    let mut count = 0;
    let mut end = 0;
    for span in spans {
        if count == 0 || span.start >= end {
            count += 1;
            end = span.end;
        } else {
            end = end.max(span.end);
        }
    }
    count
}

/// How many times `entry`'s term occurs in `text`, in any of its forms:
/// forms that overlap are one occurrence.
#[cfg(test)]
fn count_term(text: &str, entry: &GlossaryEntry) -> usize {
    count_spans(&entry_spans(&Folded::new(text), entry))
}

/// Whether the strings of `exceptions` name `entry`'s term, by any of its
/// forms, ignoring case.
fn is_exception(entry: &GlossaryEntry, exceptions: &[String]) -> bool {
    entry.is_excepted(exceptions)
}

impl Glossary {
    /// Entries whose term occurs in `text`, in any of its forms, each with
    /// how many times. An occurrence inside an occurrence of a longer term
    /// is that term's alone: `Scions of the Seventh Dawn` is not also
    /// `Scions`; forms of one term that overlap are one occurrence.
    fn occurrences(&self, text: &str) -> Vec<(&GlossaryEntry, usize)> {
        self.occurring(text)
            .into_iter()
            .filter_map(|(index, count)| Some((self.entries.get(index)?, count)))
            .collect()
    }

    /// The entries, by index, whose term occurs in `text`, each with how
    /// many times, as [`Self::occurrences`] finds them.
    fn occurring(&self, text: &str) -> Vec<(usize, usize)> {
        let text = text_of(text);
        let folded = Folded::new(&text);
        let found: Vec<(usize, Vec<std::ops::Range<usize>>)> = self
            .candidates(&folded.lower)
            .into_iter()
            .filter_map(|index| Some((index, entry_spans(&folded, self.entries.get(index)?))))
            .filter(|(_, spans)| !spans.is_empty())
            .collect();
        found
            .iter()
            .filter_map(|(index, spans)| {
                let own = count_spans(
                    spans
                        .iter()
                        .filter(|span| !found.iter().any(|(_, longer)| covered(span, longer))),
                );
                (own > 0).then_some((*index, own))
            })
            .collect()
    }

    /// Whether the term of the entry at `index` occurs in `text`, as
    /// [`Self::used_in`] finds it: not inside a longer term.
    #[must_use]
    pub fn uses(&self, text: &str, index: usize) -> bool {
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        let plain = text_of(text);
        // Most strings do not have the term at all: only those that do are
        // matched against the other terms.
        if entry_spans(&Folded::new(&plain), entry).is_empty() {
            return false;
        }
        self.occurring(text)
            .iter()
            .any(|(found, _)| *found == index)
    }

    /// The indexes of the entries whose term occurs in `text`, in order:
    /// counted over all the strings of a project, how many strings use each
    /// term.
    #[must_use]
    pub fn used_in(&self, text: &str) -> Vec<usize> {
        self.occurring(text)
            .into_iter()
            .map(|(index, _)| index)
            .collect()
    }

    /// Entries whose term occurs in `text` (see [`Self::occurrences`]).
    #[must_use]
    pub fn matches(&self, text: &str) -> Vec<&GlossaryEntry> {
        self.matches_except(text, &[])
    }

    /// Entries whose term occurs in `text`, but for the terms of
    /// `exceptions`: terms a person decided do not apply to the string.
    #[must_use]
    pub fn matches_except(&self, text: &str, exceptions: &[String]) -> Vec<&GlossaryEntry> {
        self.occurrences(text)
            .into_iter()
            .map(|(entry, _)| entry)
            .filter(|entry| !is_exception(entry, exceptions))
            .collect()
    }

    /// The strings of `exceptions` that name no term occurring in `source`:
    /// the term left the glossary, or the source changed.
    #[must_use]
    pub fn stale_exceptions<'a>(&self, source: &str, exceptions: &'a [String]) -> Vec<&'a str> {
        let found = self.matches(source);
        exceptions
            .iter()
            .filter(|exception| {
                !found
                    .iter()
                    .any(|entry| is_exception(entry, std::slice::from_ref(*exception)))
            })
            .map(String::as_str)
            .collect()
    }

    /// The entry of a term by any of its forms, ignoring case.
    #[must_use]
    pub fn find(&self, term: &str) -> Option<&GlossaryEntry> {
        let term = term.to_lowercase();
        let Some(names) = self.names() else {
            return self
                .entries
                .iter()
                .find(|entry| entry.all_forms().any(|form| form.to_lowercase() == term));
        };
        names
            .forms
            .get(&term)
            .and_then(|&index| self.entries.get(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_term_is_found_by_name_after_entries_are_added() {
        let mut glossary = Glossary::new(vec![
            sense("Gaia", "Гея", "персонаж"),
            entry("Minfilia", "Минфилия"),
        ]);
        assert_eq!(
            glossary.find("MINFILIA").map(|e| e.translation.as_str()),
            Some("Минфилия")
        );
        assert!(glossary.find("Ryne").is_none());
        glossary.append(vec![sense("gaia", "Гайя", "место"), entry("Ryne", "Рина")]);
        assert_eq!(
            glossary.find("ryne").map(|e| e.translation.as_str()),
            Some("Рина")
        );
        let senses: Vec<&str> = glossary
            .senses(&glossary.entries[0])
            .into_iter()
            .map(|sense| sense.translation.as_str())
            .collect();
        assert_eq!(senses, ["Гея", "Гайя"]);
    }

    fn entry(term: &str, translation: &str) -> GlossaryEntry {
        GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            category: "понятие".to_owned(),
            ..GlossaryEntry::default()
        }
    }

    fn sense(term: &str, translation: &str, category: &str) -> GlossaryEntry {
        GlossaryEntry {
            category: category.to_owned(),
            ..entry(term, translation)
        }
    }

    #[test]
    fn a_name_has_one_meaning_per_category() {
        let csv = "term,translation,forms,category\nMaelstrom,Мальстрём,,организация\nmaelstrom,водоворот,maelstroms,существо\nMaelstrom,Водоворот,,Существо\nAether,Эфир,maelstroms,понятие\n";
        let glossary = parse_glossary(csv.as_bytes()).expect("glossary");
        let terms: Vec<(&str, &str)> = glossary
            .entries
            .iter()
            .map(|entry| (entry.translation.as_str(), entry.category.as_str()))
            .collect();
        // Another category is another meaning; the same category, ignoring
        // case, or another name's form is a repeat.
        assert_eq!(
            terms,
            [("Мальстрём", "организация"), ("водоворот", "существо")]
        );
        let lines: Vec<u64> = glossary.diagnostics.iter().map(|d| d.line).collect();
        assert_eq!(lines, [4, 5]);
        let text = write_glossary(&glossary.entries);
        assert!(
            text.starts_with("term,translation,forms,note,category\n"),
            "{text}"
        );
        assert_eq!(
            parse_glossary(text.as_bytes()).expect("again").entries,
            glossary.entries
        );
    }

    #[test]
    fn a_term_matches_its_english_plural() {
        assert_eq!(plural("gladiator").as_deref(), Some("gladiators"));
        assert_eq!(plural("Black Mage").as_deref(), Some("Black Mages"));
        assert_eq!(plural("fairy").as_deref(), Some("fairies"));
        assert_eq!(plural("day").as_deref(), Some("days"));
        assert_eq!(
            plural("Hydaelyn's grace").as_deref(),
            Some("Hydaelyn's graces")
        );
        assert_eq!(plural("crux").as_deref(), Some("cruxes"));
        assert_eq!(plural("Эфир"), None);
        let glossary = Glossary::new(vec![
            entry("gladiator", "гладиатор"),
            GlossaryEntry {
                match_case: true,
                ..entry("Fairy", "Фея")
            },
        ]);
        assert_eq!(glossary.matches("Two gladiators.").len(), 1);
        assert_eq!(glossary.matches("The Fairies dance.").len(), 1);
        assert!(glossary.matches("The fairies dance.").is_empty());
        assert!(glossary.matches("A gladiatorial bout.").is_empty());
    }

    #[test]
    fn the_strings_that_use_each_term_are_counted() {
        let glossary = Glossary::new(vec![
            entry("Scions", "Наследники"),
            entry("Scions of the Seventh Dawn", "Наследники Седьмой Зари"),
            entry("aether", "эфир"),
        ]);
        let mut strings = vec![0; glossary.entries.len()];
        for text in [
            "The Scions of the Seventh Dawn",
            "The Scions",
            "Aether, aether everywhere",
            "Nothing",
        ] {
            for index in glossary.used_in(text) {
                strings[index] += 1;
            }
        }
        // A string counts once for a term, and not for a shorter term inside
        // a longer one.
        assert_eq!(strings, [1, 1, 1]);
        // A string uses a term as it is counted.
        assert!(glossary.uses("The Scions", 0));
        assert!(!glossary.uses("The Scions of the Seventh Dawn", 0));
        assert!(glossary.uses("The Scions of the Seventh Dawn", 1));
        assert!(!glossary.uses("Nothing", 2));
        assert!(!glossary.uses("Aether", 9));
    }

    #[test]
    fn rows_are_read_and_invalid_rows_reported() {
        let csv = "\u{feff}translation,term,forms,case,category\nЭфир,Aether,aethers; ,yes,понятие\n,Empty,,,понятие\nКристалл,Crystal,,,понятие\nКристал,crystal,,,понятие\n\n\"Мудрец, старший\",\"Sage, elder\",,,титул\nЭфиры,Aetherial,AETHERS,,понятие\nЛинк,linkshell,linkshells; Linkshell,,понятие\nПусто,Nameless,,,\n";
        let glossary = parse_glossary(csv.as_bytes()).expect("glossary");
        assert_eq!(glossary.entries.len(), 3);
        assert_eq!(glossary.entries[0].forms, vec!["aethers"]);
        assert!(glossary.entries[0].match_case);
        assert!(!glossary.entries[1].match_case);
        assert_eq!(glossary.entries[2].term, "Sage, elder");
        let lines: Vec<u64> = glossary.diagnostics.iter().map(|d| d.line).collect();
        assert_eq!(lines, [3, 5, 8, 9, 10]);
        assert!(glossary.diagnostics[1].message.contains("already defined"));
        assert!(glossary.diagnostics[2].message.contains("AETHERS"));
        assert!(glossary.diagnostics[3].message.contains("Linkshell"));
        // A term without a category is excluded.
        assert!(glossary.diagnostics[4].message.contains("category"));
    }

    #[test]
    fn a_bad_header_refuses_the_file() {
        assert!(parse_glossary(b"term,meaning\na,b\n").is_err());
        assert!(parse_glossary(b"term\na\n").is_err());
        assert!(parse_glossary(b"term,term,translation\n").is_err());
        assert!(parse_glossary(b"term,translation,forbidden\n").is_err());
        assert!(parse_glossary(b"term,translation\na,b\n").is_err());
        assert!(
            parse_glossary(b"term,translation,category\n")
                .expect("empty")
                .entries
                .is_empty()
        );
    }

    #[test]
    fn the_canonical_writer_round_trips() {
        let entries = vec![
            GlossaryEntry {
                forms: vec!["aethers".to_owned(), "the aether".to_owned()],
                note: Some("line \"one\"".to_owned()),
                match_case: true,
                ..entry("Aether", "Эфир")
            },
            entry("Sage, elder", "Мудрец"),
        ];
        let text = write_glossary(&entries);
        assert!(text.starts_with("term,translation,forms,note,case,category\n"));
        assert!(text.contains(",aethers; the aether,"));
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
            ..Glossary::default()
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
    fn terms_are_words_of_the_text_never_of_macros() {
        let glossary = Glossary {
            entries: vec![entry("aetheryte", "эфирит")],
            ..Glossary::default()
        };
        let source = r#"Return to <noun-en PlaceName 2 "<sheet Aetheryte $n1 8>" 2 1>?"#;
        assert!(
            glossary.matches(source).is_empty(),
            "a sheet name is not a word"
        );
        assert_eq!(glossary.matches("Attune to the aetheryte.").len(), 1);
        assert_eq!(
            glossary
                .matches("<if ($n1 > 0)>the aetheryte<else>none</if>")
                .len(),
            1,
            "text inside a condition is text"
        );
        assert_eq!(text_of(r"a \<b> c"), "a <b> c");
    }

    #[test]
    fn every_form_is_the_term() {
        let glossary = Glossary::new(vec![
            GlossaryEntry {
                forms: vec!["linkshells".to_owned()],
                ..entry("linkshell", "линкшелл")
            },
            GlossaryEntry {
                forms: vec!["the Scions".to_owned(), "Scions".to_owned()],
                ..entry("Scions of the Seventh Dawn", "Потомки Седьмой Зари")
            },
        ]);
        let terms = |text: &str| -> Vec<String> {
            glossary
                .matches(text)
                .iter()
                .map(|entry| entry.term.clone())
                .collect()
        };
        assert_eq!(terms("Two linkshells"), ["linkshell"]);
        assert_eq!(terms("Ask the Scions"), ["Scions of the Seventh Dawn"]);
        assert!(terms("Linkshellers").is_empty());
        // A form inside a longer form is one occurrence.
        let scions = &glossary.entries[1];
        assert_eq!(count_term("The Scions of the Seventh Dawn", scions), 1);
        assert_eq!(count_term("Scions and the Scions", scions), 2);
        assert!(glossary.find("LINKSHELLS").is_some());
        // An exception may name the term by any of its forms.
        let exceptions = vec!["linkshells".to_owned()];
        assert!(
            glossary
                .matches_except("A linkshell", &exceptions)
                .is_empty()
        );
        assert!(
            glossary
                .stale_exceptions("A linkshell", &exceptions)
                .is_empty()
        );
    }

    #[test]
    fn a_longer_term_takes_its_words_and_a_term_may_match_case() {
        let glossary = Glossary {
            entries: vec![
                entry("Scions", "Потомки"),
                entry("Scions of the Seventh Dawn", "Потомки Седьмой Зари"),
                GlossaryEntry {
                    match_case: true,
                    ..entry("the Maelstrom", "Мальстрём")
                },
            ],
            ..Glossary::default()
        };
        let terms = |text: &str| -> Vec<String> {
            glossary
                .matches(text)
                .iter()
                .map(|entry| entry.term.clone())
                .collect()
        };
        assert_eq!(
            terms("The Scions of the Seventh Dawn"),
            ["Scions of the Seventh Dawn"]
        );
        assert_eq!(
            terms("Scions of the Seventh Dawn and other Scions"),
            ["Scions", "Scions of the Seventh Dawn"]
        );
        assert_eq!(terms("The Maelstrom sails"), ["the Maelstrom"]);
        assert_eq!(terms("for the Maelstrom"), ["the Maelstrom"]);
        assert!(terms("into the maelstrom").is_empty());
        assert!(terms("THE MAELSTROM").is_empty());
    }

    #[test]
    fn exceptions_lift_a_term_and_stale_ones_are_found() {
        let glossary = Glossary {
            entries: vec![entry("Maelstrom", "Мальстрём")],
            ..Glossary::default()
        };
        let source = "Maelstrom of Despair";
        assert_eq!(glossary.matches_except(source, &[]).len(), 1);
        let exceptions = vec!["maelstrom".to_owned()];
        assert!(glossary.matches_except(source, &exceptions).is_empty());
        assert!(glossary.stale_exceptions(source, &exceptions).is_empty());
        let stale = vec!["Maelstrom".to_owned(), "Ishgard".to_owned()];
        assert_eq!(
            glossary.stale_exceptions("Nothing", &stale),
            ["Maelstrom", "Ishgard"]
        );
        assert_eq!(glossary.stale_exceptions(source, &stale), ["Ishgard"]);
    }
}
