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

/// A translation as the terms of a source are looked for in it: its text,
/// folded, and its words, each made once for all the terms.
struct Target<'a> {
    folded: Folded<'a>,
    words: Vec<String>,
}

impl<'a> Target<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            folded: Folded::new(text),
            words: words(text),
        }
    }

    /// Whether the translation uses the term of `entry` found `count` times
    /// in the source: keeps it as written, or has the words of its
    /// translation or of one of `rendered`.
    fn uses(&self, entry: &GlossaryEntry, count: usize, rendered: &[&str]) -> bool {
        kept_as_written(&self.folded, entry, count)
            || contains_inflected(&self.words, &entry.translation, rendered)
    }
}

/// Whether `folded` keeps the term as the source writes it wherever the
/// source has it (`count` times): part of a name left untranslated. A term
/// whose translation only spells it another way (`FATE` as `F.A.T.E.`) is
/// never kept, since the terms say how to write it.
fn kept_as_written(folded: &Folded<'_>, entry: &GlossaryEntry, count: usize) -> bool {
    let letters = |text: &str| -> String {
        text.chars()
            .filter(|letter| letter.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let respelled =
        entry.translation != entry.term && letters(&entry.translation) == letters(&entry.term);
    !respelled && count_spans(&entry_spans(folded, entry)) >= count
}

/// Whether the strings of `exceptions` name `entry`'s term, by any of its
/// forms, ignoring case.
fn is_exception(entry: &GlossaryEntry, exceptions: &[String]) -> bool {
    exceptions.iter().any(|exception| {
        let exception = exception.to_lowercase();
        entry
            .all_forms()
            .any(|form| form.to_lowercase() == exception)
    })
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

/// Endings of a noun in `-а` or `-я` (`Носка`, `Эорзея`, `гильдия`): its
/// cases in both numbers, so `Носке` is a form of `Носка` and `Носкее`,
/// with an ending of an adjective, is not.
const NOUN_A_ENDINGS: [&str; 20] = [
    "", "а", "я", "ы", "и", "е", "у", "ю", "й", "ь", "ой", "ей", "ою", "ею", "ам", "ям", "ами",
    "ями", "ах", "ях",
];

/// Endings of an adjective in `-ая`, `-яя`, or `-ый` (`Западная`,
/// `первобытный`): its genders, cases, and numbers, so `Западной` is a form
/// of `Западная` and `Западна`, with an ending of a noun, is not.
const ADJECTIVE_ENDINGS: [&str; 26] = [
    "ая", "яя", "ой", "ей", "ую", "юю", "ою", "ею", "ый", "ий", "ого", "его", "ому", "ему", "ым",
    "им", "ом", "ем", "ое", "ее", "ые", "ие", "ых", "их", "ыми", "ими",
];

/// The endings a word declines with, by the letters its stem leaves off:
/// a noun in `-а` or `-я` and an adjective in `-ая`, `-яя`, or `-ый` take
/// only their own; any other word, whose ending does not tell a noun from an
/// adjective (`гений`, `знание`), takes any of [`ENDINGS`].
fn endings_after(trimmed: &str) -> &'static [&'static str] {
    match trimmed {
        "ая" | "яя" | "ый" => &ADJECTIVE_ENDINGS,
        _ if trimmed.ends_with(['а', 'я']) => &NOUN_A_ENDINGS,
        _ => &ENDINGS,
    }
}

/// Most letters an inflected form adds to a stem.
const MAX_ENDING: usize = 4;

/// The stem of a lowercase word: without its final vowels and signs, but at
/// least two letters of a word of three or more (`эра`, `эр`; `око`, `ок`;
/// `Гея`, `ге`), so a short word declines too.
fn stem(word: &str) -> &str {
    let trimmed = word.trim_end_matches(|c| ENDING_LETTERS.contains(c));
    let (kept, length) = (trimmed.chars().count(), word.chars().count());
    if kept >= 2 && length >= 3 || kept >= 3 {
        trimmed
    } else if length >= 3 {
        word.char_indices()
            .nth(length - 1)
            .map_or(word, |(index, _)| &word[..index])
    } else {
        word
    }
}

/// Endings of the nouns in `-мя` after their stem: `пламя`, `пламени`;
/// `племя`, `племён` (written here with `е`, as [`is_form`] compares).
const MYA_ENDINGS: [&str; 7] = ["ени", "енем", "ена", "ен", "енам", "енами", "енах"];

/// Whether `candidate` is `word` as written or inflected, both lowercase.
/// `ё` and `е` are one letter, since a text may write either. A word in
/// `-ся` declines before it (`клубящиеся`, `клубящихся`), and a noun in
/// `-мя` takes `-ен-` (`пламя`, `пламени`).
fn is_form(candidate: &str, word: &str) -> bool {
    let without_yo = |text: &str| {
        text.chars()
            .next()
            .map(|first| if first == 'ё' { 'е' } else { first })
    };
    // Every form begins as its word does: most words of a text are not
    // compared further.
    if without_yo(candidate) != without_yo(word) {
        return false;
    }
    let (candidate, word) = (yo_as_e(candidate), yo_as_e(word));
    // A word as written is its own form, whatever letter it ends with
    // (`Сигурэ`, `XII`).
    if candidate == word {
        return true;
    }
    // A word written with hyphens declines part by part (`Лимса-Ломинса`,
    // `Лимсу-Ломинсу`) and has as many parts in each of its forms.
    if candidate.contains('-') || word.contains('-') {
        let (candidates, parts) = (candidate.split('-'), word.split('-'));
        return candidate.matches('-').count() == word.matches('-').count()
            && candidates
                .zip(parts)
                .all(|(candidate, part)| is_form(candidate, part));
    }
    if let (Some(candidate), Some(word)) = (candidate.strip_suffix("ся"), word.strip_suffix("ся"))
        && word.chars().count() >= 3
    {
        let stem = stem(word);
        return inflects(candidate, stem, &word[stem.len()..]);
    }
    let stem = stem(&word);
    inflects(&candidate, stem, &word[stem.len()..])
        || (word.ends_with("мя")
            && candidate
                .strip_prefix(stem)
                .is_some_and(|ending| MYA_ENDINGS.contains(&ending)))
}

/// `text` with `ё` written as `е`, copied only when it has one.
fn yo_as_e(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('ё') {
        std::borrow::Cow::Owned(text.replace('ё', "е"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// The lowercase words of a text, in order. An apostrophe or a hyphen
/// between letters is part of its word, so `Уль'ды` is one word, a form of
/// `Уль'да`, and `Ла-Носке` is one word, not a form of `Ла Носка`.
fn words(text: &str) -> Vec<String> {
    written_words(&text.to_lowercase())
        .into_iter()
        .map(|(word, _)| word)
        .collect()
}

/// What a word may follow to begin with a capital letter: the end of a
/// sentence, a line break, which [`cased_text`] writes for each tag, or a
/// quotation mark, which opens a title.
const CAPITAL_AFTER: [char; 9] = ['.', '!', '?', '…', '\n', '«', '„', '“', '"'];

/// The words of a text as it writes them, in order, each with whether a
/// sentence or a title may begin at it: it is the first word, or what stands
/// between it and the word before has a mark of [`CAPITAL_AFTER`].
fn written_words(text: &str) -> Vec<(String, bool)> {
    let letters: Vec<char> = text.chars().collect();
    let in_word = |at: usize| {
        is_word(letters[at])
            || ((APOSTROPHES.contains(&letters[at]) || HYPHENS.contains(&letters[at]))
                && at > 0
                && is_word(letters[at - 1])
                && letters.get(at + 1).copied().is_some_and(is_word))
    };
    let mut words = Vec::new();
    let mut word = String::new();
    let mut begins = true;
    for (at, letter) in letters.iter().enumerate() {
        if in_word(at) {
            // One spelling of the apostrophe and of the hyphen, whichever
            // the text writes.
            word.push(if is_word(*letter) {
                *letter
            } else if HYPHENS.contains(letter) {
                '-'
            } else {
                '\''
            });
            continue;
        }
        if !word.is_empty() {
            words.push((std::mem::take(&mut word), begins));
            begins = false;
        }
        begins |= CAPITAL_AFTER.contains(letter);
    }
    if !word.is_empty() {
        words.push((word, begins));
    }
    words
}

/// Apostrophes a name is written with.
const APOSTROPHES: [char; 3] = ['\'', '’', 'ʼ'];

/// Hyphens a word is written with: `Ла-Носка`.
const HYPHENS: [char; 3] = ['-', '\u{2010}', '\u{2011}'];

/// Whether `word` is an inflected form of a word with this stem, which
/// leaves `trimmed` off the word: the stem with an ending, or the stem with
/// or without a vowel that comes and goes when the word is declined
/// (`боец`, `бойца`; `девушка`, `девушек`).
fn inflects(word: &str, stem: &str, trimmed: &str) -> bool {
    declines(word, stem, trimmed)
        || fleeting_stems(stem)
            .iter()
            .any(|stem| declines(word, stem, trimmed))
        || inserts_vowel(word, stem)
}

/// Whether `word` is `stem` with an ending of declension, one of those
/// [`endings_after`] gives for `trimmed`. Before the ending it may take back
/// part of what the stem left off the word (`чарод` + `е` + `я` of
/// `чародей`) or `ь` and `й` (`ноч` + `ь` + `ю`), but not all of a final
/// vowel: an ending takes the vowel's place, so `Ализея` is not a form of
/// `Ализе`.
fn declines(word: &str, stem: &str, trimmed: &str) -> bool {
    let Some(ending) = word.strip_prefix(stem) else {
        return false;
    };
    let endings = endings_after(trimmed);
    ending.chars().count() <= MAX_ENDING
        && ending
            .char_indices()
            .map(|(index, _)| index)
            .chain([ending.len()])
            .any(|split| {
                let taken = &ending[..split];
                let signs = taken.chars().all(|c| "ьй".contains(c));
                let part = taken.len() < trimmed.len() && trimmed.starts_with(taken);
                (signs || part) && endings.contains(&&ending[split..])
            })
}

/// Vowels of Russian.
const VOWELS: &str = "аеёиоуыэюя";

/// Vowels that come and go before a word's last consonant when it is
/// declined.
const FLEETING: [char; 3] = ['о', 'е', 'ё'];

/// The stems a word may decline from when the vowel before its last
/// consonant drops: `мореглядец` gives `мореглядц`, `замок` `замк`, `камен`
/// `камн`, `хребет` `хребт`, `церков` `церкв`. The vowel may leave `й` after
/// a vowel (`боец`, `бойц`) and `ь` after a consonant (`владелец`,
/// `владельц`; `огонек`, `огоньк`), so both stems are given. None for a
/// stem that ends another way, or would be shorter than three letters.
fn fleeting_stems(stem: &str) -> Vec<String> {
    let letters: Vec<char> = stem.chars().collect();
    let [head @ .., before, vowel, last] = letters.as_slice() else {
        return Vec::new();
    };
    if !FLEETING.contains(vowel) || !"цкнлртв".contains(*last) {
        return Vec::new();
    }
    let with = |left: Option<char>| -> String {
        head.iter()
            .chain([before])
            .chain(left.iter())
            .chain([last])
            .collect()
    };
    let stems = if VOWELS.contains(*before) {
        vec![with(Some('й'))]
    } else {
        vec![with(None), with(Some('ь'))]
    };
    stems
        .into_iter()
        .filter(|stem| stem.chars().count() >= 3)
        .collect()
}

/// Whether `word` is `stem` with a vowel between its last two consonants
/// and no ending, as a genitive plural is: `девушк` gives `девушек`, `сестр`
/// `сестёр`, `земл` `земель`; the vowel takes the place of `ь` or `й`
/// (`кольц`, `колец`).
fn inserts_vowel(word: &str, stem: &str) -> bool {
    let word = word.strip_suffix('ь').unwrap_or(word);
    let letters: Vec<char> = stem.chars().collect();
    let [head @ .., before, last] = letters.as_slice() else {
        return false;
    };
    if head.is_empty() || ENDING_LETTERS.contains(*last) || VOWELS.contains(*before) {
        return false;
    }
    let kept = (!"ьй".contains(*before)).then_some(before);
    FLEETING.iter().any(|vowel| {
        word.chars()
            .eq(head.iter().chain(kept).chain([vowel, last]).copied())
    })
}

/// Whether the words of `translation` appear in `target`, the words of a
/// text, one after another, in order, allowing for inflected endings: each
/// is the word's stem with an ending, so `эфирита` uses `эфирит` and
/// `эфироита` does not, and `списке целей ... задания` does not use
/// `список заданий`. A word of `rendered` stands for the whole translation
/// while the target has it, as written or inflected.
fn contains_inflected(target: &[String], translation: &str, rendered: &[&str]) -> bool {
    let has = |phrase: &[String]| {
        !phrase.is_empty()
            && target.windows(phrase.len()).any(|run| {
                run.iter()
                    .zip(phrase)
                    .all(|(candidate, word)| is_form(candidate, word))
            })
    };
    has(&words(translation)) || rendered.iter().any(|form| has(&words(form)))
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
    /// [`Self::count_uses`] counts it: not inside a longer term.
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

    /// Adds to `strings`, by the index of each entry, one for every entry
    /// whose term occurs in `text`: counted over all the strings of a
    /// project, how many strings use each term.
    pub fn count_uses(&self, text: &str, strings: &mut [usize]) {
        for (index, _) in self.occurring(text) {
            if let Some(count) = strings.get_mut(index) {
                *count += 1;
            }
        }
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

    /// What the glossary finds in a translation of `source`, but for the
    /// terms of `exceptions`, with the terms of the source found once: see
    /// [`Self::unused_terms`] and [`Self::stale_exceptions`]. `rendered`
    /// names, by a term's headword, words that render the term in this
    /// string: a term counts as used while the translation has one.
    #[must_use]
    pub fn review<'a, 'e>(
        &'a self,
        source: &str,
        target: &str,
        exceptions: &'e [String],
        rendered: &[(&str, &str)],
    ) -> TermReview<'a, 'e> {
        let found = self.occurrences(source);
        let target = text_of(target);
        let target = Target::new(&target);
        // A name is used when the translation of one of its meanings is.
        let used: Vec<bool> = found
            .iter()
            .map(|(entry, count)| {
                let rendered: Vec<&str> = rendered
                    .iter()
                    .filter(|(term, _)| term.to_lowercase() == entry.term.to_lowercase())
                    .map(|(_, word)| *word)
                    .collect();
                target.uses(entry, *count, &rendered)
            })
            .collect();
        let mut review = TermReview::default();
        for (index, (entry, _)) in found.iter().enumerate() {
            if is_exception(entry, exceptions) {
                continue;
            }
            let meanings: Vec<usize> = (0..found.len())
                .filter(|other| found[*other].0.same_name(entry))
                .collect();
            if meanings.first() == Some(&index) && meanings.iter().all(|other| !used[*other]) {
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
        let target = Target::new(&target);
        let found = self.occurrences(source);
        let used: Vec<bool> = found
            .iter()
            .map(|(entry, count)| target.uses(entry, *count, &[]))
            .collect();
        // A name is used when one of its meanings is; it is told once.
        found
            .iter()
            .enumerate()
            .filter(|(index, (entry, _))| {
                let meanings: Vec<usize> = (0..found.len())
                    .filter(|other| found[*other].0.same_name(entry))
                    .collect();
                !is_exception(entry, exceptions)
                    && meanings.first() == Some(index)
                    && meanings.iter().all(|other| !used[*other])
            })
            .map(|(_, (entry, _))| *entry)
            .collect()
    }

    /// The translations of the meanings of the name of `entry`, each once,
    /// joined with ` / `: what a translation of the name may use.
    #[must_use]
    pub fn translations_of(&self, entry: &GlossaryEntry) -> String {
        let mut translations: Vec<&str> = Vec::new();
        for sense in self.senses(entry) {
            if !translations.contains(&sense.translation.as_str()) {
                translations.push(&sense.translation);
            }
        }
        if translations.is_empty() {
            entry.translation.clone()
        } else {
            translations.join(" / ")
        }
    }

    /// Terms of the source whose translation does not seem to be used, as
    /// messages.
    #[must_use]
    pub fn missing_in(&self, source: &str, target: &str) -> Vec<String> {
        self.unused_terms(source, target, &[])
            .into_iter()
            .map(|entry| {
                format!(
                    "the terms translate {:?} as {:?}, which does not seem to be used",
                    entry.term,
                    self.translations_of(entry)
                )
            })
            .collect()
    }
}

/// What [`Glossary::review`] finds in a translation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TermReview<'a, 'e> {
    /// Terms of the source whose translation does not seem to be used.
    pub unused: Vec<&'a GlossaryEntry>,
    /// Exceptions that name no term of the source.
    pub stale: Vec<&'e str>,
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

    #[test]
    fn a_translation_is_used_as_written_or_inflected() {
        let glossary = Glossary::new(vec![
            entry("Minfilia", "Минфилия"),
            entry("Scions of the Seventh Dawn", "Потомки Седьмой Зари"),
        ]);
        assert!(
            glossary
                .missing_in("Ask Minfilia.", "Спросите <i>Минфилию</i>.")
                .is_empty()
        );
        assert!(
            glossary
                .missing_in("The Scions of the Seventh Dawn", "у Потомков Седьмой Зари")
                .is_empty()
        );
        assert_eq!(
            glossary.missing_in("Ask Minfilia.", "Спросите Крил.").len(),
            1
        );
    }

    #[test]
    fn a_word_declines_only_with_endings_of_its_kind() {
        let glossary = Glossary::new(vec![entry("Western La Noscea", "Западная Ла Носка")]);
        let source = "Visit western La Noscea.";
        for used in [
            "в Западной Ла Носке",
            "Западную Ла Носку",
            "над Западными Ла Носками",
        ] {
            assert!(glossary.missing_in(source, used).is_empty(), "{used}");
        }
        for unused in [
            "в Западной Ла Носкее",
            "в Западной Ла-Носке",
            "в Западной Ла-Носкее",
            "в Западна Ла Носке",
        ] {
            assert_eq!(glossary.missing_in(source, unused).len(), 1, "{unused}");
        }
        assert!(is_form("эорзее", "эорзея"));
        assert!(is_form("гильдией", "гильдия"));
        assert!(is_form("гильдий", "гильдия"));
        assert!(!is_form("носком", "носка"));
        assert!(is_form("первобытных", "первобытный"));
        assert!(!is_form("первобытна", "первобытный"));
    }

    #[test]
    fn a_hyphenated_word_is_one_word() {
        let glossary = Glossary::new(vec![entry("Limsa Lominsa", "Лимса-Ломинса")]);
        let source = "Return to Limsa Lominsa.";
        assert!(glossary.missing_in(source, "в Лимсу-Ломинсу").is_empty());
        assert_eq!(glossary.missing_in(source, "в Лимсу Ломинсу").len(), 1);
        assert_eq!(words("Ла‑Носка и Ла-Носка"), ["ла-носка", "и", "ла-носка"]);
        assert_eq!(words("слово - другое"), ["слово", "другое"]);
    }

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

    #[test]
    fn the_words_of_a_translation_are_used_together() {
        let glossary = Glossary::new(vec![entry("Duty List", "Список заданий")]);
        let source = "Open the journal or duty list.";
        assert_eq!(
            glossary
                .missing_in(
                    source,
                    "Чтобы принять задания, откройте журнал или списке целей.",
                )
                .len(),
            1
        );
        assert_eq!(
            glossary
                .missing_in(source, "Заданий список откройте.")
                .len(),
            1
        );
        assert!(
            glossary
                .missing_in(source, "Откройте журнал или список заданий.")
                .is_empty()
        );
        assert!(
            glossary
                .missing_in(source, "Откройте журнал или <i>списке заданий</i>.")
                .is_empty()
        );
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
    fn a_translation_of_any_meaning_uses_the_name() {
        let glossary = Glossary::new(vec![
            sense("Mist", "Туман", "место"),
            sense("Mist", "туман", "существо"),
            sense("Maelstrom", "Мальстрём", "организация"),
            sense("Maelstrom", "водоворот", "существо"),
        ]);
        assert!(
            glossary
                .missing_in("Into the maelstrom", "В водоворот")
                .is_empty()
        );
        assert!(glossary.missing_in("The Maelstrom", "Мальстрём").is_empty());
        let missing = glossary.missing_in("The Maelstrom", "Буря");
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("Мальстрём / водоворот"), "{missing:?}");
        // The case of one meaning is enough: «туман» is the creature.
        let review = glossary.review("A mist rises", "Встаёт туман", &[], &[]);
        assert!(review.unused.is_empty(), "{review:?}");
        // An exception names every meaning of the name.
        let exceptions = ["Maelstrom".to_owned()];
        assert!(
            glossary
                .unused_terms("The Maelstrom", "Буря", &exceptions)
                .is_empty()
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
            glossary.count_uses(text, &mut strings);
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
    fn checks_accept_inflections_and_tell_a_term_from_a_similar_word() {
        let glossary = Glossary::new(vec![
            entry("Aether", "Эфир"),
            entry("aetheryte", "эфирит"),
            entry("conjurer", "чародей"),
        ]);
        assert!(
            glossary
                .missing_in("The aether flows", "Эфиром полон мир")
                .is_empty()
        );
        assert_eq!(
            glossary.missing_in("The aether flows", "Этер течёт").len(),
            1
        );
        let source = "Limsa Lominsa Aetheryte Plaza";
        // «эфироита» shares «эфир» with «эфирит», but is another word.
        assert_eq!(glossary.missing_in(source, "Площадь эфироита").len(), 1);
        assert!(glossary.missing_in(source, "Площадь эфирита").is_empty());
        assert!(
            glossary
                .missing_in("The conjurer waits", "Чародея ждут")
                .is_empty()
        );
        let glossary = Glossary::new(vec![
            entry("Alisaie", "Ализе"),
            entry("night", "ночь"),
            entry("Eorzea", "Эорзея"),
            entry("thicket", "чаща"),
        ]);
        // An ending takes the place of a final vowel, never follows it.
        assert_eq!(glossary.missing_in("Alisaie", "Ализея").len(), 1);
        assert!(glossary.missing_in("Alisaie", "Ализе").is_empty());
        // A word as written is used, whatever it ends with.
        let names = Glossary::new(vec![
            entry("Shigure", "Сигурэ"),
            entry("12th Legion laquearius", "лаквеарий XII легиона"),
        ]);
        assert!(names.missing_in("Shigure", "Сигурэ").is_empty());
        assert!(
            names
                .missing_in("12th Legion laquearius", "лаквеарий XII легиона")
                .is_empty()
        );
        for (source, target) in [
            ("At night", "Ночью"),
            ("All of Eorzea", "По всей Эорзее"),
            ("Thickets", "За чащами"),
        ] {
            assert!(glossary.missing_in(source, target).is_empty(), "{target}");
        }
    }

    #[test]
    fn checks_accept_a_vowel_that_comes_and_goes() {
        let glossary = Glossary::new(vec![
            entry("Seagazer", "Мореглядец"),
            entry("fighter", "боец"),
            entry("owner", "владелец"),
            entry("spark", "огонёк"),
            entry("stone", "камень"),
            entry("castle", "замок"),
            entry("girl", "девушка"),
            entry("ring", "кольцо"),
            entry("land", "земля"),
            entry("city", "город"),
        ]);
        for (source, target) in [
            ("Seagazer Shoals", "Отмели Мореглядца"),
            ("A fighter", "Бойцу"),
            ("The owner", "Владельцем"),
            ("A spark", "Огоньки"),
            ("The stone", "Камнями"),
            ("The castle", "В замке"),
            ("A girl", "Пять девушек"),
            ("A ring", "Семь колец"),
            ("The land", "Много земель"),
        ] {
            assert!(glossary.missing_in(source, target).is_empty(), "{target}");
        }
        // Another word, not a form: no vowel drops before «д».
        assert_eq!(glossary.missing_in("The city", "Гордый").len(), 1);
        assert_eq!(glossary.missing_in("The castle", "Замкнут").len(), 1);
        let glossary = Glossary::new(vec![
            entry("Immortal Flames", "Бессмертное пламя"),
            entry("beast tribes", "племена зверолюдов"),
            entry("the Churning Mists", "Клубящиеся туманы"),
            entry("Astral Era", "Астральная эра"),
            entry("the Howling Eye", "Воющее око"),
            entry("Gaia", "Гея"),
            entry("Abalathia's Spine", "Хребет Абалатии"),
            entry("the Church", "Церковь"),
            entry("spark", "огонёк"),
        ]);
        for (source, target) in [
            ("Immortal Flames Standard", "Знамя Бессмертного пламени"),
            ("The beast tribes", "На пути племён зверолюдов"),
            ("In the Churning Mists", "В Клубящихся туманах"),
            ("The Sixth Astral Era", "Шестой Астральной эры"),
            ("In the Howling Eye", "В Воющем оке"),
            ("With Gaia", "С Геей и о Гее"),
            ("Abalathia's Spine", "У Хребта Абалатии"),
            ("At the Church", "У Церкви"),
            ("A spark", "Два огонька"),
        ] {
            assert!(glossary.missing_in(source, target).is_empty(), "{target}");
        }
        assert_eq!(glossary.missing_in("The Astral Era", "Эрцгерцог").len(), 1);
    }

    #[test]
    fn a_term_spelled_another_way_is_not_kept_as_written() {
        let glossary = Glossary::new(vec![entry("FATE", "F.A.T.E.")]);
        let source = "Complete the FATE \"A Finale Most Formidable\".";
        assert_eq!(
            glossary.missing_in(source, "Завершить FATE «Финал».").len(),
            1
        );
        assert!(
            glossary
                .missing_in(source, "Завершить F.A.T.E. «Финал».")
                .is_empty()
        );
    }

    #[test]
    fn a_word_made_from_a_translation_renders_its_term_where_named() {
        let glossary = Glossary::new(vec![
            entry("aether", "эфир"),
            entry("Scions of the Seventh Dawn", "Потомки Седьмой Зари"),
        ]);
        let unused = |target: &str, rendered: &[(&str, &str)]| {
            glossary
                .review("Aether Compressor", target, &[], rendered)
                .unused
                .len()
        };
        assert_eq!(unused("Эфирный компрессор", &[]), 1);
        assert_eq!(unused("Эфирный компрессор", &[("Aether", "Эфирный")]), 0);
        // The named word declines with the translation.
        assert_eq!(unused("Эфирного компрессора", &[("aether", "эфирный")]), 0);
        // A word the translation lost, or of another term, renders nothing.
        assert_eq!(unused("Сжиматель", &[("aether", "эфирный")]), 1);
        assert_eq!(unused("Эфирный компрессор", &[("mount", "эфирный")]), 1);
        // One word renders a term of several.
        let review = glossary.review(
            "The Scions of the Seventh Dawn",
            "Потомковый дом",
            &[],
            &[("Scions of the Seventh Dawn", "потомковый")],
        );
        assert!(review.unused.is_empty());
    }

    #[test]
    fn a_translation_is_used_in_any_case() {
        let glossary = Glossary::new(vec![
            entry("malm", "мальм"),
            entry("Dawn Throne", "Трон Рассвета"),
        ]);
        // The terms give a name as it stands alone; the sentence writes its
        // capitals.
        assert!(
            glossary
                .missing_in("One malm away.", "В одном Мальме отсюда.")
                .is_empty()
        );
        assert!(
            glossary
                .missing_in("The Dawn Throne", "Это трон рассвета")
                .is_empty()
        );
    }

    #[test]
    fn an_apostrophe_between_letters_is_part_of_its_word() {
        let glossary = Glossary::new(vec![entry("Ul'dah", "Уль'да"), entry("mi'qote", "ми’коте")]);
        let source = "Ul'dah Aetheryte Plaza";
        assert!(
            glossary
                .missing_in(source, "Площадь эфирита Уль'ды")
                .is_empty()
        );
        assert!(
            glossary
                .missing_in(source, "Площадь эфирита Уль’ды")
                .is_empty()
        );
        // Without the apostrophe it is another spelling of the name.
        assert_eq!(
            glossary.missing_in(source, "Площадь эфирита Ульды").len(),
            1
        );
        assert!(glossary.missing_in("A mi'qote", "'Ми'коте'").is_empty());
        assert_eq!(words("'да' и Уль'ды’"), ["да", "и", "уль'ды"]);
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
        assert!(
            glossary
                .missing_in("Join the Scions", "Вступите к Потомкам Седьмой Зари")
                .is_empty()
        );
        assert_eq!(
            glossary.missing_in("Your linkshells", "Ваши каналы").len(),
            1
        );
        assert!(glossary.find("LINKSHELLS").is_some());
        // An exception may name the term by any of its forms.
        let exceptions = vec!["linkshells".to_owned()];
        assert!(
            glossary
                .unused_terms("A linkshell", "Связь", &exceptions)
                .is_empty()
        );
        assert!(
            glossary
                .stale_exceptions("A linkshell", &exceptions)
                .is_empty()
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
            entries: vec![entry("Maelstrom", "Мальстрём")],
            ..Glossary::default()
        };
        let source = "Maelstrom of Despair";
        let target = "Водоворот отчаяния";
        assert_eq!(glossary.unused_terms(source, target, &[]).len(), 1);
        let exceptions = vec!["maelstrom".to_owned()];
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
        let review = glossary.review(source, target, &stale, &[]);
        assert!(review.unused.is_empty());
        assert_eq!(review.stale, ["Ishgard"]);
        let review = glossary.review(source, target, &[], &[]);
        assert_eq!(review.unused.len(), 1);
    }
}
