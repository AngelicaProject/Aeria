//! The project's terms: `aeria-knowledge/terms.csv`, Glossary Format v1
//! (`docs/formats/glossary-v1.md`).

use serde::Serialize;

/// Largest terms file read.
pub const MAX_GLOSSARY_BYTES: u64 = 8 * 1024 * 1024;
/// Most entries read.
pub const MAX_GLOSSARY_ENTRIES: usize = 100_000;

const COLUMNS: [&str; 6] = [
    "term",
    "translation",
    "note",
    "forbidden",
    "settled",
    "case",
];
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
    /// The term matches only with its case as written, but for a capital
    /// first letter: `the Maelstrom` matches `The Maelstrom`, not
    /// `the maelstrom`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub match_case: bool,
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
}

impl PartialEq for Glossary {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries && self.diagnostics == other.diagnostics
    }
}

impl Eq for Glossary {}

/// The lowercase terms of a glossary in one automaton, which finds in a
/// lowercase text every term that may occur, overlapping ones too. Most
/// strings have no term, and a string's terms are then matched one by one.
#[derive(Clone, Debug)]
struct Finder {
    automaton: Option<aho_corasick::AhoCorasick>,
    /// The entries the automaton was made for: entries added since are
    /// matched one by one.
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

    /// Indexes of the entries whose term may occur in `lower`, the lowercase
    /// text, in order.
    fn candidates(&self, lower: &str) -> Vec<usize> {
        let finder = self.finder.get_or_init(|| Finder {
            automaton: aho_corasick::AhoCorasick::builder()
                .match_kind(aho_corasick::MatchKind::Standard)
                .build(self.entries.iter().map(|entry| entry.term.to_lowercase()))
                .ok(),
            entries: self.entries.len(),
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
            .map(|found| found.pattern().as_usize())
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

/// Whether a `settled` or `case` field is set.
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
            settled: is_yes(field(4)),
            match_case: is_yes(field(5)),
        });
    }
    Ok(glossary)
}

/// Writes entries in the canonical form: the full header (`case` only when
/// an entry matches case), one entry per line, LF line endings, and quoting
/// only where needed.
#[must_use]
pub fn write_glossary(entries: &[GlossaryEntry]) -> String {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    // `case` is written only when an entry uses it, so a file without it
    // stays as it was.
    let columns = if entries.iter().any(|entry| entry.match_case) {
        COLUMNS.len()
    } else {
        COLUMNS.len() - 1
    };
    let _ = writer.write_record(&COLUMNS[..columns]);
    for entry in entries {
        let forbidden = entry.forbidden.join("; ");
        let record = [
            entry.term.as_str(),
            entry.translation.as_str(),
            entry.note.as_deref().unwrap_or_default(),
            forbidden.as_str(),
            if entry.settled { "yes" } else { "" },
            if entry.match_case { "yes" } else { "" },
        ];
        let _ = writer.write_record(&record[..columns]);
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
                text.push(' ');
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

/// How many times `entry`'s term occurs in `text`.
fn count_term(text: &str, entry: &GlossaryEntry) -> usize {
    Folded::new(text).spans(&entry.term, entry.match_case).len()
}

/// Whether the strings of `exceptions` name `entry`'s term, ignoring case.
fn is_exception(entry: &GlossaryEntry, exceptions: &[String]) -> bool {
    let term = entry.term.to_lowercase();
    exceptions
        .iter()
        .any(|exception| exception.to_lowercase() == term)
}

/// Letters an inflected ending replaces at the end of a word: vowels and
/// the soft and short signs, so the stem of `заклинатель` is `заклинател`,
/// of `первобытный` `первобытн`, and of `эфирит` the word itself.
const ENDING_LETTERS: &str = "аеёиоуыэюяйьіїєaeiouy";

/// Endings of declension that follow a stem after its trimmed letters:
/// `заклинател` + `ями`, `первобытн` + `ого`, `чарод` + `е` + `я`. Any other
/// letters make another word: `этер` + `ис` is `Этерис`, not a form of `этер`.
const ENDINGS: [&str; 52] = [
    "", "а", "е", "ё", "и", "о", "у", "ы", "ю", "я", "й", "ь", "і", "ї", "є", "м", "х", "в", "ом",
    "ем", "ём", "ой", "ей", "ою", "ею", "ам", "ям", "ах", "ях", "ов", "ев", "ми", "ий", "ый", "ая",
    "яя", "ое", "ее", "ые", "ие", "их", "ых", "им", "ым", "ую", "юю", "ого", "его", "ому", "ему",
    "ами", "ями",
];

/// Most letters an inflected form adds to a stem.
const MAX_ENDING: usize = 4;

/// The stem of a lowercase word: without its final vowels and signs, but at
/// least three letters long.
fn stem(word: &str) -> &str {
    let trimmed = word.trim_end_matches(|c| ENDING_LETTERS.contains(c));
    if trimmed.chars().count() >= 3 {
        trimmed
    } else {
        word.char_indices()
            .nth(3)
            .map_or(word, |(index, _)| &word[..index])
    }
}

/// The lowercase words of a text, in order.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !is_word(c))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether `word` is an inflected form of a word with this stem.
fn inflects(word: &str, stem: &str) -> bool {
    let Some(ending) = word.strip_prefix(stem) else {
        return false;
    };
    ending.chars().count() <= MAX_ENDING
        && ending
            .char_indices()
            .map(|(index, _)| index)
            .chain([ending.len()])
            .any(|split| {
                ending[..split].chars().all(|c| ENDING_LETTERS.contains(c))
                    && ENDINGS.contains(&&ending[split..])
            })
}

/// Whether every word of `translation` appears in `target`, allowing for
/// inflected endings: some word of the target is the word's stem with an
/// ending, so `эфирита` uses `эфирит` and `эфироита` does not.
fn contains_inflected(target: &str, translation: &str) -> bool {
    let target = words(target);
    words(translation).iter().all(|word| {
        let stem = stem(word);
        target.iter().any(|candidate| inflects(candidate, stem))
    })
}

/// How many times `translation` occurs in `target` with each of its words
/// as written or inflected, in order: `Потомков Седьмой Зари` is one use of
/// `Потомки Седьмой Зари`.
fn count_inflected(target: &str, translation: &str) -> usize {
    let stems: Vec<String> = words(translation)
        .iter()
        .map(|word| stem(word).to_owned())
        .collect();
    if stems.is_empty() {
        return 0;
    }
    words(target)
        .windows(stems.len())
        .filter(|window| {
            window
                .iter()
                .zip(&stems)
                .all(|(word, stem)| inflects(word, stem))
        })
        .count()
}

/// Whether `target` uses a forbidden variant: as written, or, for a variant
/// of words of at least four letters, in an inflected form (`эфироит` also
/// forbids `эфироита`). A variant with other characters (`призыв.`) or a
/// shorter word (`нин`) matches only as written.
fn contains_variant(target: &str, variant: &str) -> bool {
    if contains_term(target, variant) {
        return true;
    }
    if variant.chars().any(|c| !is_word(c) && !c.is_whitespace()) {
        return false;
    }
    let parts = words(variant);
    if parts.is_empty() || parts.iter().any(|part| part.chars().count() < 4) {
        return false;
    }
    let stems: Vec<&str> = parts.iter().map(|part| stem(part)).collect();
    let target = words(target);
    target.windows(stems.len()).any(|window| {
        window
            .iter()
            .zip(&stems)
            .all(|(word, stem)| inflects(word, stem))
    })
}

impl Glossary {
    /// Entries whose term occurs in `text`, each with how many times. An
    /// occurrence inside an occurrence of a longer term is that term's
    /// alone: `Scions of the Seventh Dawn` is not also `Scions`.
    fn occurrences(&self, text: &str) -> Vec<(&GlossaryEntry, usize)> {
        let text = text_of(text);
        let folded = Folded::new(&text);
        let found: Vec<(&GlossaryEntry, Vec<std::ops::Range<usize>>)> = self
            .candidates(&folded.lower)
            .into_iter()
            .filter_map(|index| self.entries.get(index))
            .map(|entry| (entry, folded.spans(&entry.term, entry.match_case)))
            .filter(|(_, spans)| !spans.is_empty())
            .collect();
        found
            .iter()
            .enumerate()
            .filter_map(|(index, (entry, spans))| {
                let own = spans
                    .iter()
                    .filter(|span| {
                        !found.iter().enumerate().any(|(other, (_, longer))| {
                            other != index
                                && longer.iter().any(|longer| {
                                    longer.len() > span.len()
                                        && longer.start <= span.start
                                        && span.end <= longer.end
                                })
                        })
                    })
                    .count();
                (own > 0).then_some((*entry, own))
            })
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

    /// The entry of a term, ignoring case.
    #[must_use]
    pub fn find(&self, term: &str) -> Option<&GlossaryEntry> {
        let term = term.to_lowercase();
        self.entries
            .iter()
            .find(|entry| entry.term.to_lowercase() == term)
    }

    /// Forbidden variants a translation uses for terms of its source, each
    /// with its entry, but for the terms of `exceptions`, while a term is
    /// translated fewer times than the source has it. A translation with one
    /// is wrong.
    #[must_use]
    pub fn forbidden_variants<'a>(
        &'a self,
        source: &str,
        target: &str,
        exceptions: &[String],
    ) -> Vec<(&'a GlossaryEntry, &'a str)> {
        self.review(source, target, exceptions).forbidden
    }

    /// What the glossary finds in a translation of `source`, but for the
    /// terms of `exceptions`, with the terms of the source found once: see
    /// [`Self::forbidden_variants`], [`Self::unused_terms`], and
    /// [`Self::stale_exceptions`].
    #[must_use]
    pub fn review<'a, 'e>(
        &'a self,
        source: &str,
        target: &str,
        exceptions: &'e [String],
    ) -> TermReview<'a, 'e> {
        let found = self.occurrences(source);
        let target = text_of(target);
        let mut review = TermReview::default();
        for (entry, count) in &found {
            if is_exception(entry, exceptions) {
                continue;
            }
            // A forbidden variant is a problem while the term is not
            // translated as often as the source has it: once every use is
            // the term's translation, the variant is another word, such as
            // `наследник` for an heir beside `Потомки` for the Scions.
            if count_inflected(&target, &entry.translation) < *count {
                for forbidden in &entry.forbidden {
                    if contains_variant(&target, forbidden) {
                        review.forbidden.push((entry, forbidden.as_str()));
                    }
                }
            }
            if count_term(&target, entry) < *count
                && !contains_inflected(&target, &entry.translation)
            {
                review.unused.push(entry);
            }
        }
        review.stale = exceptions
            .iter()
            .filter(|exception| {
                !found
                    .iter()
                    .any(|(entry, _)| is_exception(entry, std::slice::from_ref(*exception)))
            })
            .map(String::as_str)
            .collect();
        review
    }

    /// Forbidden variants a translation uses for terms of its source, as
    /// messages. A translation with one is wrong.
    #[must_use]
    pub fn forbidden_in(&self, source: &str, target: &str) -> Vec<String> {
        self.forbidden_variants(source, target, &[])
            .into_iter()
            .map(|(entry, forbidden)| forbidden_message(entry, forbidden))
            .collect()
    }

    /// Entries of terms of the source whose translation does not seem to be
    /// used, but for the terms of `exceptions`. Only advice: inflected
    /// languages rarely keep the dictionary form, and a line may render a
    /// term another way on purpose. A term the translation keeps as written
    /// wherever the source has it is part of a name left untranslated
    /// (`Scions & Sinners`), not a term to translate.
    #[must_use]
    pub fn unused_terms(
        &self,
        source: &str,
        target: &str,
        exceptions: &[String],
    ) -> Vec<&GlossaryEntry> {
        let target = text_of(target);
        self.occurrences(source)
            .into_iter()
            .filter(|(entry, count)| {
                !is_exception(entry, exceptions)
                    && count_term(&target, entry) < *count
                    && !contains_inflected(&target, &entry.translation)
            })
            .map(|(entry, _)| entry)
            .collect()
    }

    /// Terms of the source whose translation does not seem to be used, as
    /// messages.
    #[must_use]
    pub fn missing_in(&self, source: &str, target: &str) -> Vec<String> {
        self.unused_terms(source, target, &[])
            .into_iter()
            .map(unused_message)
            .collect()
    }
}

/// What [`Glossary::review`] finds in a translation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TermReview<'a, 'e> {
    /// Forbidden variants the translation uses, with their entries.
    pub forbidden: Vec<(&'a GlossaryEntry, &'a str)>,
    /// Terms of the source whose translation does not seem to be used.
    pub unused: Vec<&'a GlossaryEntry>,
    /// Exceptions that name no term of the source.
    pub stale: Vec<&'e str>,
}

/// The message of a forbidden variant a translation uses.
#[must_use]
pub fn forbidden_message(entry: &GlossaryEntry, forbidden: &str) -> String {
    format!(
        "the terms forbid {forbidden:?} for {:?}; use {:?}",
        entry.term, entry.translation
    )
}

/// The message of a term whose translation does not seem to be used.
#[must_use]
pub fn unused_message(entry: &GlossaryEntry) -> String {
    format!(
        "the terms translate {:?} as {:?}, which does not seem to be used",
        entry.term, entry.translation
    )
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
                match_case: true,
                ..entry("Aether", "Эфир")
            },
            entry("Sage, elder", "Мудрец"),
        ];
        let text = write_glossary(&entries);
        assert!(text.starts_with("term,translation,note,forbidden,settled,case\n"));
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
            entries: vec![GlossaryEntry {
                forbidden: vec!["Этерит".to_owned()],
                ..entry("aetheryte", "эфирит")
            }],
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
        assert!(
            glossary
                .unused_terms(
                    "Attune to the aetheryte.",
                    "<sheet Aetheryte 1 0> эфирит",
                    &[]
                )
                .is_empty()
        );
        assert_eq!(text_of(r"a \<b> c"), "a <b> c");
    }

    #[test]
    fn checks_accept_inflections_and_find_forbidden_variants() {
        let glossary = Glossary {
            entries: vec![GlossaryEntry {
                forbidden: vec!["Этер".to_owned()],
                ..entry("Aether", "Эфир")
            }],
            ..Glossary::default()
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

    #[test]
    fn stems_tell_a_term_from_a_similar_word_and_inflected_variants_are_forbidden() {
        let glossary = Glossary {
            entries: vec![
                GlossaryEntry {
                    forbidden: vec!["эфироит".to_owned(), "нин".to_owned()],
                    ..entry("aetheryte", "эфирит")
                },
                GlossaryEntry {
                    forbidden: vec!["заклинатель".to_owned(), "повелитель зверей".to_owned()],
                    ..entry("conjurer", "чародей")
                },
            ],
            ..Glossary::default()
        };
        let source = "Limsa Lominsa Aetheryte Plaza";
        // «эфироита» shares «эфир» with «эфирит», but is another word.
        assert_eq!(glossary.missing_in(source, "Площадь эфироита").len(), 1);
        assert!(glossary.missing_in(source, "Площадь эфирита").is_empty());
        assert_eq!(glossary.forbidden_in(source, "Площадь эфироита").len(), 1);
        assert!(glossary.forbidden_in(source, "Площадь эфирита").is_empty());
        // Short variants match only as written: «нина» is not «нин».
        assert!(glossary.forbidden_in(source, "Нина у эфирита").is_empty());
        let conjurer = "The conjurer waits";
        assert_eq!(glossary.forbidden_in(conjurer, "Ждёт заклинателя").len(), 1);
        assert_eq!(
            glossary
                .forbidden_in(conjurer, "Повелителя зверей нет")
                .len(),
            1
        );
        assert!(glossary.missing_in(conjurer, "Чародея ждут").is_empty());
        // A name that starts with a forbidden variant is another word.
        let aether = Glossary::new(vec![GlossaryEntry {
            forbidden: vec!["этер".to_owned()],
            ..entry("aether", "эфир")
        }]);
        assert!(
            aether
                .forbidden_in("The aether of Etheirys", "Эфир Этериса")
                .is_empty()
        );
        assert!(
            aether
                .forbidden_in("The aether of Etheirys", "Эфир Этерисе")
                .is_empty()
        );
        assert_eq!(
            aether.forbidden_in("The aether flows", "Этера много").len(),
            1
        );
        assert_eq!(
            aether
                .forbidden_in("The aether flows", "Этером полон")
                .len(),
            1
        );
        // Once the term is translated wherever the source has it, a variant
        // is another word: an heir beside the Scions.
        let scions = Glossary::new(vec![GlossaryEntry {
            forbidden: vec!["Наследники".to_owned()],
            ..entry("Scions", "Потомки")
        }]);
        let source = "The Scions welcome the heir.";
        assert!(
            scions
                .forbidden_in(source, "Потомки встречают наследника.")
                .is_empty()
        );
        assert_eq!(
            scions
                .forbidden_in(source, "Наследники встречают наследника.")
                .len(),
            1
        );
        assert_eq!(
            scions
                .forbidden_in("Scions and Scions", "Потомки и Наследники")
                .len(),
            1,
            "one use of two"
        );
        assert!(
            glossary
                .forbidden_in(conjurer, "Заклинательная сила")
                .is_empty(),
            "more than an ending"
        );
    }

    #[test]
    fn a_term_kept_as_written_is_part_of_an_untranslated_name() {
        let glossary = Glossary {
            entries: vec![entry("Scions", "Потомки")],
            ..Glossary::default()
        };
        let source = "Orchestrion Roll (Scions & Sinners: Band)";
        assert!(
            glossary
                .missing_in(source, "Свиток оркестриона (Scions & Sinners: Band)")
                .is_empty()
        );
        // Kept once, but the source has the term twice.
        assert_eq!(
            glossary
                .missing_in(
                    "The Scions hum Scions & Sinners",
                    "Напевают «Scions & Sinners»"
                )
                .len(),
            1
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
            entries: vec![GlossaryEntry {
                forbidden: vec!["водоворот".to_owned()],
                ..entry("Maelstrom", "Мальстрём")
            }],
            ..Glossary::default()
        };
        let source = "Maelstrom of Despair";
        let target = "Водоворот отчаяния";
        assert_eq!(glossary.forbidden_variants(source, target, &[]).len(), 1);
        assert_eq!(glossary.unused_terms(source, target, &[]).len(), 1);
        let exceptions = vec!["maelstrom".to_owned()];
        assert!(
            glossary
                .forbidden_variants(source, target, &exceptions)
                .is_empty()
        );
        assert!(
            glossary
                .unused_terms(source, target, &exceptions)
                .is_empty()
        );
        assert!(glossary.stale_exceptions(source, &exceptions).is_empty());
        let stale = vec!["Maelstrom".to_owned(), "Ishgard".to_owned()];
        assert_eq!(
            glossary.stale_exceptions("Nothing", &stale),
            ["Maelstrom", "Ishgard"]
        );
        let review = glossary.review(source, target, &stale);
        assert!(review.forbidden.is_empty() && review.unused.is_empty());
        assert_eq!(review.stale, ["Ishgard"]);
        let review = glossary.review(source, target, &[]);
        assert_eq!((review.forbidden.len(), review.unused.len()), (1, 1));
    }
}
