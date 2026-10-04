//! Glossary candidates: names of the English sources that the project's
//! translations render in several ways, such as `Kojin` as «Кодзин»,
//! «Кудзин», and «Койдзин». A person decides what goes into the glossary;
//! this only finds where the project disagrees with itself.
//!
//! A *candidate* is a capitalized phrase of the sources (`Twelveswood`,
//! `Students of Baldesion`, `Radz-at-Han`), not at the start of a sentence,
//! in enough strings, mostly outside interface sheets, and not mostly inside
//! a longer candidate. A *rendering* is a Russian word of the translations
//! of those strings that is far more frequent there than in the project and
//! occurs mostly in them, so it is the candidate's translation rather than a
//! word around it; its forms (Кодзина, Кодзину) are one rendering. Words
//! that share their strings are parts of one rendering («Лимсы Ломинсы»).
//! Two renderings that rarely share a string are rivals, and a candidate
//! with rivals is reported. See `docs/architecture/search.md`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use aeria_knowledge::Knowledge;

use crate::po::PoFile;
use crate::project::{PO_DIR, list};
use crate::search::{SearchError, text_ranges};

/// Strings a candidate needs, translated ones among them.
const MIN_STRINGS: u64 = 12;
/// Words of the translations a rendering is looked for among, most frequent
/// first.
const TOP_WORDS: usize = 60;
/// Strings a rendering needs.
const MIN_RENDERING: u64 = 4;
/// Examples kept for each rendering.
const EXAMPLES: usize = 3;
/// Most words of a candidate.
const MAX_WORDS: usize = 4;

/// Sheets of interface labels, whose words are capitalized as labels
/// rather than as names: a candidate needs strings outside them.
const INTERFACE_SHEETS: &[&str] = &[
    "Action",
    "ActionTransient",
    "Addon",
    "AddonTransient",
    "BannerBg",
    "BannerDesignPreset",
    "BannerFrame",
    "BaseParam",
    "ClassJob",
    "CompanionTransient",
    "ConfigKey",
    "ContentFinderConditionTransient",
    "ContentsTutorialPage",
    "CraftAction",
    "DescriptionString",
    "Error",
    "GilShop",
    "Glasses",
    "GlassesStyle",
    "HWDDevLevelWebText",
    "HowTo",
    "HowToPage",
    "Item",
    "ItemUICategory",
    "Lobby",
    "LogMessage",
    "MainCommand",
    "MonsterNote",
    "MountTransient",
    "Ornament",
    "SpecialShop",
    "Status",
    "TextCommand",
    "Trait",
    "TraitTransient",
    "Tutorial",
    "XBMItem",
];

/// Words that are capitalized for the sentence or as function words, never
/// the start of a name.
const STOP_WORDS: &[&str] = &[
    "A", "After", "Ah", "All", "Although", "And", "Any", "Are", "As", "At", "Be", "Been", "Before",
    "Being", "Both", "But", "By", "Can", "Could", "Did", "Do", "Does", "Down", "Each", "Either",
    "Even", "Every", "For", "From", "Had", "Has", "Have", "He", "Her", "Here", "His", "How", "I",
    "If", "In", "Into", "Is", "It", "Just", "Let", "Lv", "May", "Me", "Might", "Must", "My",
    "Neither", "No", "Not", "Now", "Of", "Off", "Oh", "On", "Once", "Only", "Or", "Our", "Out",
    "Over", "Please", "Shall", "She", "Should", "So", "Some", "Still", "Such", "Than", "Thank",
    "Thanks", "That", "The", "Then", "There", "These", "They", "This", "Those", "Though", "To",
    "Under", "Up", "Us", "Very", "Was", "We", "Well", "Were", "What", "When", "Where", "While",
    "Why", "Will", "With", "Would", "Yes", "You", "Your",
];

/// Lowercase words that may join the words of a name: `Students of
/// Baldesion`, `Scions of the Seventh Dawn`, `Costa del Sol`.
const JOINERS: &[&str] = &["of", "the", "de", "del"];

/// Russian endings, longest first: a word without one is its rendering's
/// form-independent part.
const ENDINGS: &[&str] = &[
    "ами", "ями", "ого", "его", "ому", "ему", "ой", "ей", "ом", "ем", "ам", "ям", "ах", "ях", "ов",
    "ев", "ия", "ию", "ии", "ие", "ье", "ья", "ью", "ьи", "ий", "ый", "ая", "яя", "ое", "ее", "ую",
    "юю", "ым", "им", "ых", "их", "а", "я", "ы", "и", "у", "ю", "е", "о", "ь",
];

/// A string where a rendering is used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Example {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    pub source: String,
    pub translation: String,
}

/// One way the project renders a candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rendering {
    /// The rendering's words as the translations most often write them,
    /// such as «Кодзин» or «Великой компании».
    pub words: Vec<String>,
    /// Translated strings that use it.
    pub strings: usize,
    pub examples: Vec<Example>,
}

/// A name the project renders in several ways.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub phrase: String,
    /// Strings whose source has it.
    pub strings: usize,
    /// Of them, translated strings.
    pub translated: usize,
    /// The most used rendering first, then its rivals by strings.
    pub renderings: Vec<Rendering>,
    /// Every rendering sounds like the name: they are spellings of one
    /// name (Кодзин, Кудзин), not translations of it with other words
    /// (Чащоба, Двенадцатилесье), which a person reviews more closely.
    pub spellings: bool,
    /// The sheets with most of its strings, with their counts.
    pub sheets: Vec<(String, usize)>,
}

/// What one entry contributes.
struct Facts {
    sheet: String,
    context: String,
    phrases: Vec<String>,
    /// `(form-independent word, word as written)` of the translation;
    /// `None` when untranslated.
    words: Option<Vec<(String, String)>>,
}

/// The sheet of a file: its first folder, or its name without `.po`.
fn sheet_of(path: &str) -> &str {
    path.split('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".po")
}

fn is_name_word(word: &str) -> bool {
    let mut parts = word.split('-');
    let first = parts.next().unwrap_or_default();
    first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && first
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'' || c == '’')
        && parts.all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c == '\'' || c == '’')
        })
}

/// The capitalized phrases of `text`, a run of plain text of a source,
/// each found once. The first word of a sentence is capitalized anyway, so
/// a phrase there keeps only the words after it.
fn phrases_of(text: &str, found: &mut HashSet<String>) {
    // Words with whether a sentence starts at them.
    let mut tokens: Vec<(&str, bool)> = Vec::new();
    let mut sentence_start = true;
    let mut word_start = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (index, &(at, c)) in chars.iter().enumerate() {
        let in_word = c.is_ascii_alphabetic()
            || ((c == '\'' || c == '’' || c == '-')
                && word_start.is_some()
                && chars
                    .get(index + 1)
                    .is_some_and(|(_, next)| next.is_ascii_alphabetic()));
        match (in_word, word_start) {
            (true, None) => word_start = Some(at),
            (false, Some(start)) => {
                tokens.push((&text[start..at], sentence_start));
                sentence_start = false;
                word_start = None;
            }
            _ => {}
        }
        if !in_word && word_start.is_none() {
            if matches!(
                c,
                '.' | '!' | '?' | ':' | ';' | '"' | '«' | '“' | '—' | '\n'
            ) {
                sentence_start = true;
            } else if c.is_ascii_digit() {
                sentence_start = false;
            }
        }
    }
    if let Some(start) = word_start {
        tokens.push((&text[start..], sentence_start));
    }
    let mut index = 0;
    while index < tokens.len() {
        let (word, _) = tokens[index];
        if !is_name_word(word) {
            index += 1;
            continue;
        }
        // The phrase: names, joined by at most two joiners each.
        let mut words = vec![tokens[index]];
        let mut next = index + 1;
        while words.iter().filter(|(w, _)| is_name_word(w)).count() < MAX_WORDS {
            let mut ahead = next;
            while ahead < tokens.len() && ahead - next < 2 && JOINERS.contains(&tokens[ahead].0) {
                ahead += 1;
            }
            if ahead < tokens.len() && is_name_word(tokens[ahead].0) && !tokens[ahead].1 {
                words.extend_from_slice(&tokens[next..=ahead]);
                next = ahead + 1;
            } else {
                break;
            }
        }
        index = next;
        let mut start = 0;
        if words[0].1 {
            start = 1;
        }
        while start < words.len()
            && (STOP_WORDS.contains(&words[start].0) || JOINERS.contains(&words[start].0))
        {
            start += 1;
        }
        let mut end = words.len();
        while end > start && JOINERS.contains(&words[end - 1].0) {
            end -= 1;
        }
        if start < end {
            let phrase = words[start..end]
                .iter()
                .map(|(w, _)| *w)
                .collect::<Vec<_>>()
                .join(" ");
            if phrase.len() >= 3 {
                found.insert(phrase);
            }
        }
    }
}

/// A Russian word without its ending (and a reflexive `-ся`), lowercase,
/// `ё` as `е`: the part its forms share.
fn base_of(word: &str) -> String {
    let mut word = word.to_lowercase().replace('ё', "е");
    for reflexive in ["ся", "сь"] {
        if word.chars().count() > 5 && word.ends_with(reflexive) {
            word.truncate(word.len() - reflexive.len());
            break;
        }
    }
    for ending in ENDINGS {
        if word.ends_with(ending) && word.chars().count() - ending.chars().count() >= 3 {
            word.truncate(word.len() - ending.len());
            break;
        }
    }
    word
}

/// The Russian words of a translation's text, three letters or more.
fn words_of(text: &str) -> Vec<(String, String)> {
    let mut words: Vec<(String, String)> = text
        .split(|c: char| !(c.is_alphabetic() && matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')))
        .filter(|word| word.chars().count() >= 3)
        .map(|word| (base_of(word), word.to_owned()))
        .collect();
    words.sort();
    words.dedup_by(|a, b| a.0 == b.0);
    words
}

/// The consonants of a word as they sound, for telling a Russian spelling
/// of a name from a translation of it: `Kojin` and «Кодзин» are both `kdzn`.
fn sound_of(word: &str) -> String {
    let mut sound = String::new();
    for c in word.to_lowercase().chars() {
        let mapped = match c {
            'b' | 'б' => "b",
            'v' | 'w' | 'в' => "v",
            'g' | 'г' => "g",
            'd' | 'д' => "d",
            'j' => "dz",
            'z' | 'з' | 'ж' => "z",
            'c' | 'k' | 'q' | 'к' => "k",
            'l' | 'л' => "l",
            'm' | 'м' => "m",
            'n' | 'н' => "n",
            'p' | 'п' => "p",
            'r' | 'р' => "r",
            's' | 'с' | 'ц' | 'ч' | 'ш' | 'щ' => "s",
            't' | 'т' => "t",
            'f' | 'ф' => "f",
            'x' => "ks",
            _ => "",
        };
        for letter in mapped.chars() {
            if !sound.ends_with(letter) {
                sound.push(letter);
            }
        }
    }
    sound
}

/// Whether a Russian word sounds like a name: their first consonants are
/// the same, and of their first four consonants three, or all of the
/// shorter, follow each other in both (Sharlayan, Шаллаян).
fn sounds_like(name: &str, word: &str) -> bool {
    let name: Vec<char> = sound_of(name).chars().take(4).collect();
    let word: Vec<char> = sound_of(word).chars().take(4).collect();
    if name.len() < 2 || word.len() < 2 || name[0] != word[0] {
        return false;
    }
    // The longest common subsequence of the two.
    let mut table = vec![vec![0_usize; word.len() + 1]; name.len() + 1];
    for (i, a) in name.iter().enumerate() {
        for (j, b) in word.iter().enumerate() {
            table[i + 1][j + 1] = if a == b {
                table[i][j] + 1
            } else {
                table[i][j + 1].max(table[i + 1][j])
            };
        }
    }
    table[name.len()][word.len()] >= 3.min(name.len()).min(word.len())
}

/// Whether two bases are forms of one word: they differ only at the end.
/// Other spellings differ in the middle (шарлаян, шаллаян).
fn same_word(a: &str, b: &str) -> bool {
    let (short, long) = if a.chars().count() <= b.chars().count() {
        (a, b)
    } else {
        (b, a)
    };
    let common = short
        .chars()
        .zip(long.chars())
        .take_while(|(x, y)| x == y)
        .count();
    let short_len = short.chars().count();
    common + 1 >= short_len && long.chars().count() - short_len <= 3
}

fn plain_text(macro_text: &str) -> String {
    let mut text = String::new();
    for range in text_ranges(macro_text) {
        text.push_str(&macro_text[range]);
        text.push('\n');
    }
    text
}

fn read_facts(root: &Path, path: &str) -> Result<Vec<Facts>, SearchError> {
    let full = root.join(PO_DIR).join(path);
    let text = std::fs::read_to_string(&full).map_err(|error| SearchError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let sheet = sheet_of(path).to_owned();
    Ok(PoFile::parse(&text)
        .0
        .entries
        .iter()
        .map(|entry| {
            let mut found = HashSet::new();
            let source = plain_text(&entry.source);
            phrases_of(&source, &mut found);
            let whole = source.trim();
            let mut phrases: Vec<String> = found.into_iter().filter(|p| p != whole).collect();
            phrases.sort();
            Facts {
                sheet: sheet.clone(),
                context: entry.context.clone(),
                phrases,
                words: (!entry.translation.is_empty())
                    .then(|| words_of(&plain_text(&entry.translation))),
            }
        })
        .collect())
}

/// Every entry's facts, files read several at a time, in path order.
fn read_all(root: &Path) -> Result<Vec<(String, Vec<Facts>)>, SearchError> {
    let mut paths = list(root).map_err(|error| SearchError::Read {
        path: String::new(),
        message: error.to_string(),
    })?;
    paths.sort();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Vec<Facts>)>> = Mutex::new(Vec::new());
    let failure: Mutex<Option<SearchError>> = Mutex::new(None);
    let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(paths.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(index) else { return };
                    match read_facts(root, path) {
                        Ok(facts) => results
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push((index, facts)),
                        Err(error) => {
                            failure
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .get_or_insert(error);
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = failure
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        return Err(error);
    }
    let mut results = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    results.sort_by_key(|(index, _)| *index);
    Ok(results
        .into_iter()
        .map(|(index, facts)| (paths[index].clone(), facts))
        .collect())
}

/// A rendering being found: its bases and the strings (indexes into the
/// candidate's translated strings) that use it.
struct Group {
    bases: Vec<String>,
    strings: HashSet<usize>,
}

/// The candidates of the project at `root` whose translations disagree,
/// those affecting most strings first. Terms of the glossary are left out.
///
/// # Errors
///
/// Returns an error when `po/` or a file cannot be read.
#[allow(clippy::too_many_lines)] // one pass over the project's strings
pub fn term_candidates(root: &Path, knowledge: &Knowledge) -> Result<Vec<Candidate>, SearchError> {
    let files = read_all(root)?;
    let glossary: HashSet<String> = knowledge
        .terms
        .entries
        .iter()
        .map(|entry| {
            let term = entry.term.trim().to_lowercase();
            term.strip_prefix("the ").unwrap_or(&term).to_owned()
        })
        .collect();

    // Every entry with where it is.
    let entries: Vec<(&str, &Facts)> = files
        .iter()
        .flat_map(|(path, facts)| facts.iter().map(move |fact| (path.as_str(), fact)))
        .collect();
    let translated = entries
        .iter()
        .filter(|(_, fact)| fact.words.is_some())
        .count() as u64;

    let mut strings: HashMap<&str, u64> = HashMap::new();
    let mut narrative: HashMap<&str, u64> = HashMap::new();
    let mut inside: HashMap<&str, u64> = HashMap::new();
    let mut sheets: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    let mut word_strings: HashMap<&str, u64> = HashMap::new();
    let mut spellings: HashMap<&str, HashMap<&str, u64>> = HashMap::new();
    for (_, fact) in &entries {
        let interface = INTERFACE_SHEETS.contains(&fact.sheet.as_str());
        for phrase in &fact.phrases {
            *strings.entry(phrase).or_default() += 1;
            if !interface {
                *narrative.entry(phrase).or_default() += 1;
            }
            *sheets
                .entry(phrase)
                .or_default()
                .entry(fact.sheet.as_str())
                .or_default() += 1;
            let longer = fact.phrases.iter().any(|other| {
                other.len() > phrase.len()
                    && other
                        .split(' ')
                        .collect::<Vec<_>>()
                        .windows(phrase.split(' ').count())
                        .any(|window| window.join(" ") == *phrase)
            });
            if longer {
                *inside.entry(phrase).or_default() += 1;
            }
        }
        for (base, written) in fact.words.iter().flatten() {
            *word_strings.entry(base).or_default() += 1;
            *spellings
                .entry(base)
                .or_default()
                .entry(written)
                .or_default() += 1;
        }
    }

    let chosen: HashSet<&str> = strings
        .iter()
        .filter(|(phrase, count)| {
            **count >= MIN_STRINGS
                && narrative.get(*phrase).copied().unwrap_or(0) * 2 >= MIN_STRINGS
                && inside.get(*phrase).copied().unwrap_or(0) * 5 < **count * 4
                && {
                    let lower = phrase.to_lowercase();
                    !glossary.contains(lower.strip_prefix("the ").unwrap_or(&lower))
                }
        })
        .map(|(phrase, _)| *phrase)
        .collect();
    let mut docs: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, (_, fact)) in entries.iter().enumerate() {
        if fact.words.is_none() {
            continue;
        }
        for phrase in &fact.phrases {
            if chosen.contains(phrase.as_str()) {
                docs.entry(phrase).or_default().push(index);
            }
        }
    }

    let mut found: Vec<(u64, Candidate, Vec<Vec<usize>>)> = Vec::new();
    for (phrase, strings_with) in &docs {
        let total = strings_with.len() as u64;
        if total < MIN_STRINGS {
            continue;
        }
        let bases_of = |index: usize| -> HashSet<&str> {
            entries[index]
                .1
                .words
                .iter()
                .flatten()
                .map(|(base, _)| base.as_str())
                .collect()
        };
        let mut local: HashMap<&str, u64> = HashMap::new();
        for &index in strings_with {
            for base in bases_of(index) {
                *local.entry(base).or_default() += 1;
            }
        }
        let mut frequent: Vec<(&str, u64)> = local.into_iter().collect();
        frequent.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        frequent.truncate(TOP_WORDS);
        // Renderings: frequent here, rare elsewhere, mostly here.
        let renderings: Vec<(&str, u64)> = frequent
            .into_iter()
            .filter(|(base, count)| {
                let everywhere = word_strings.get(base).copied().unwrap_or(u64::MAX);
                *count >= MIN_RENDERING
                    && *count * 20 >= total
                    && *count * translated >= 30 * everywhere * total
                    && *count * 10 >= everywhere * 3
            })
            .collect();
        // Forms of one word are one rendering.
        let mut words: Vec<(String, Vec<&str>)> = Vec::new();
        for (base, _) in &renderings {
            match words.iter_mut().find(|(first, _)| same_word(first, base)) {
                Some((_, forms)) => forms.push(base),
                None => words.push(((*base).to_owned(), vec![base])),
            }
        }
        let form_of: HashMap<&str, &str> = words
            .iter()
            .flat_map(|(first, forms)| forms.iter().map(move |form| (*form, first.as_str())))
            .collect();
        let uses: Vec<(String, HashSet<usize>)> = words
            .iter()
            .map(|(first, _)| {
                let strings: HashSet<usize> = strings_with
                    .iter()
                    .enumerate()
                    .filter(|(_, index)| {
                        bases_of(**index)
                            .iter()
                            .any(|base| form_of.get(base) == Some(&first.as_str()))
                    })
                    .map(|(position, _)| position)
                    .collect();
                (first.clone(), strings)
            })
            .collect();
        // Words that share their strings are one rendering.
        let mut groups: Vec<Group> = Vec::new();
        for (word, strings) in uses {
            let shared = groups.iter_mut().find(|group| {
                let together = group.strings.intersection(&strings).count();
                together * 5 >= group.strings.len().min(strings.len()) * 3
            });
            match shared {
                Some(group) => {
                    group.bases.push(word);
                    group.strings.extend(strings);
                }
                None => groups.push(Group {
                    bases: vec![word],
                    strings,
                }),
            }
        }
        groups.retain(|group| group.strings.len() as u64 >= MIN_RENDERING);
        groups.sort_by_key(|group| std::cmp::Reverse(group.strings.len()));
        let Some((top, others)) = groups.split_first() else {
            continue;
        };
        let rivals: Vec<&Group> = others
            .iter()
            .filter(|group| {
                let overlap = group.strings.intersection(&top.strings).count();
                overlap * 20 <= group.strings.len() * 3
                    && group.strings.len() as u64 >= MIN_RENDERING
                    && group.strings.len() as u64 * 20 >= total
            })
            .collect();
        if rivals.is_empty() {
            continue;
        }
        let affected: u64 = rivals.iter().map(|group| group.strings.len() as u64).sum();
        let spelled = |base: &str| -> String {
            words
                .iter()
                .find(|(first, _)| first == base)
                .map(|(_, forms)| forms.as_slice())
                .unwrap_or_default()
                .iter()
                .filter_map(|form| spellings.get(form))
                .flat_map(|written| written.iter())
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map_or_else(|| base.to_owned(), |(written, _)| (*written).to_owned())
        };
        let shown: Vec<&Group> = std::iter::once(top).chain(rivals).collect();
        // A rendering is named by its word that sounds like the name, if
        // any: «Бутик» rather than the words of a description around it.
        let name_word = phrase
            .split(' ')
            .next()
            .unwrap_or(phrase)
            .replace(['\'', '’'], "");
        let alike = |base: &str| sounds_like(&name_word, base);
        let spellings = shown
            .iter()
            .all(|group| group.bases.iter().any(|base| alike(base)));
        let mut where_from: Vec<(String, usize)> = sheets
            .get(phrase)
            .map(|by| {
                by.iter()
                    .map(|(sheet, n)| ((*sheet).to_owned(), *n))
                    .collect()
            })
            .unwrap_or_default();
        where_from.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        where_from.truncate(3);
        let mut example_strings = Vec::new();
        let renderings = shown
            .iter()
            .map(|group| {
                let mut chosen: Vec<usize> =
                    group.strings.iter().map(|p| strings_with[*p]).collect();
                chosen.sort_unstable();
                chosen.truncate(EXAMPLES);
                example_strings.push(chosen);
                let mut bases: Vec<&String> = group.bases.iter().collect();
                bases.sort_by_key(|base| !alike(base));
                Rendering {
                    words: bases.into_iter().map(|base| spelled(base)).collect(),
                    strings: group.strings.len(),
                    examples: Vec::new(),
                }
            })
            .collect();
        found.push((
            affected,
            Candidate {
                phrase: (*phrase).to_owned(),
                strings: usize::try_from(strings.get(phrase).copied().unwrap_or(0))
                    .unwrap_or(usize::MAX),
                translated: strings_with.len(),
                renderings,
                spellings,
                sheets: where_from,
            },
            example_strings,
        ));
    }
    // Spellings of a name first: they are almost always real.
    found.sort_by(|a, b| {
        b.1.spellings
            .cmp(&a.1.spellings)
            .then_with(|| b.0.cmp(&a.0))
            .then_with(|| a.1.phrase.cmp(&b.1.phrase))
    });

    // Examples are read again from their files: the pass above keeps no
    // texts.
    let mut wanted: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (_, _, examples) in &found {
        for &index in examples.iter().flatten() {
            let (path, fact) = entries[index];
            wanted
                .entry(path)
                .or_default()
                .insert(fact.context.as_str());
        }
    }
    let mut texts: HashMap<(String, String), (String, String)> = HashMap::new();
    for (path, contexts) in wanted {
        let text = std::fs::read_to_string(root.join(PO_DIR).join(path)).map_err(|error| {
            SearchError::Read {
                path: path.to_owned(),
                message: error.to_string(),
            }
        })?;
        for entry in PoFile::parse(&text).0.entries {
            if contexts.contains(entry.context.as_str()) {
                texts.insert(
                    (path.to_owned(), entry.context),
                    (entry.source, entry.translation),
                );
            }
        }
    }
    Ok(found
        .into_iter()
        .map(|(_, mut candidate, examples)| {
            for (rendering, chosen) in candidate.renderings.iter_mut().zip(examples) {
                rendering.examples = chosen
                    .into_iter()
                    .filter_map(|index| {
                        let (path, fact) = entries[index];
                        let (source, translation) =
                            texts.get(&(path.to_owned(), fact.context.clone()))?;
                        Some(Example {
                            path: path.to_owned(),
                            context: fact.context.clone(),
                            source: source.clone(),
                            translation: translation.clone(),
                        })
                    })
                    .collect();
            }
            candidate
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phrases(text: &str) -> Vec<String> {
        let mut found = HashSet::new();
        phrases_of(text, &mut found);
        let mut found: Vec<String> = found.into_iter().collect();
        found.sort();
        found
    }

    #[test]
    fn phrases_are_names_not_sentence_starts() {
        assert_eq!(
            phrases("The Students of Baldesion met at Radz-at-Han. Kojin trade there."),
            ["Radz-at-Han", "Students of Baldesion"]
        );
        assert_eq!(
            phrases("You and Y'shtola went to the Twelveswood."),
            ["Twelveswood", "Y'shtola"]
        );
        assert_eq!(phrases("Scions of the Seventh Dawn"), ["Seventh Dawn"]);
        assert_eq!(
            phrases("He met the Scions of the Seventh Dawn"),
            ["Scions of the Seventh Dawn"]
        );
    }

    #[test]
    fn forms_of_one_word_share_a_base_and_spellings_do_not() {
        assert_eq!(base_of("Кодзинов"), base_of("Кодзина"));
        assert_eq!(base_of("возвышающихся"), base_of("возвышающиеся"));
        assert!(same_word(&base_of("Эорзея"), &base_of("Эорзее")));
        assert!(!same_word(&base_of("Шарлаян"), &base_of("Шаллаян")));
        assert!(!same_word(&base_of("Кодзин"), &base_of("Кудзин")));
    }

    #[test]
    fn spellings_of_a_name_sound_like_it_and_translations_do_not() {
        for word in ["кодзин", "кудзин", "койджин"] {
            assert!(sounds_like("Kojin", word), "{word}");
        }
        assert!(sounds_like("Boutique", "бутик"));
        assert!(sounds_like("Sharlayan", "шаллаян"));
        assert!(!sounds_like("Boutique", "чудаковатых"));
        assert!(!sounds_like("Twelveswood", "чащоба"));
    }
}
