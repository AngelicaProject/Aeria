//! The phrasing of the project's translations: the phrases they open
//! sentences with, such as «Похоже,» or «Ну что,», how many strings use
//! each, which words of the sources come with it, and how often none of
//! them does. A phrase the translations open many sentences with, for one
//! source word, reads as a habit of the translator; a phrase most of whose
//! uses have no word of the source behind it was added. A person decides
//! what the style says about them; this only counts.
//!
//! An *opener* is one to three words at the start of a sentence of a
//! translation, followed by a comma, a dash, or an ellipsis. Names are left
//! out: a word the translations mostly write with a capital inside a
//! sentence. A *cue* of an opener is a word of the sources far more frequent
//! in its strings than in the project's: found from the project itself,
//! never listed, so it works for any pair of languages. See
//! `docs/architecture/search.md`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::candidates::{Example, plain_text, read_each};
use crate::po::PoFile;
use crate::project::PO_DIR;
use crate::search::SearchError;

/// Strings an opener needs to be reported.
const MIN_STRINGS: usize = 50;
/// Most openers reported, most strings first.
const MAX_OPENERS: usize = 80;
/// Most cues of an opener.
const MAX_CUES: usize = 4;
/// How much more frequent a cue is in an opener's strings than in the
/// project's.
const MIN_LIFT: f64 = 6.0;
/// The share of an opener's strings a cue needs.
const MIN_CUE_SHARE: f64 = 0.03;
/// Examples kept of an opener's strings without a cue.
const EXAMPLES: usize = 3;

/// A phrase the translations open sentences with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Opener {
    /// In lower case, without its punctuation: `похоже`, `ну что`.
    pub phrase: String,
    /// Translated strings with a sentence that opens with it.
    pub strings: usize,
    /// The words of the sources that come with it, most telling first.
    pub cues: Vec<Cue>,
    /// Its strings whose source has none of the cues.
    pub unsupported: usize,
    /// Strings whose source has none of the cues.
    pub examples: Vec<Example>,
}

/// A word of the sources that comes with an opener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cue {
    pub word: String,
    /// The opener's strings whose source has it.
    pub strings: usize,
}

/// The openers of a project.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Phrasing {
    /// Translated strings read.
    pub translated: usize,
    pub openers: Vec<Opener>,
}

/// What phrasing needs of one translated string.
#[derive(Clone, Debug, Default)]
pub struct Pair {
    /// The file, relative to `po/`.
    pub path: String,
    pub context: String,
    /// Macro text as the file has it, for examples.
    pub source: String,
    pub translation: String,
    /// The words of the source, in lower case.
    pub source_words: HashSet<String>,
    /// The openers of the translation's sentences.
    pub openers: HashSet<String>,
    /// Words of the translation inside a sentence, in lower case, with
    /// whether each was written with a capital.
    pub inner_words: Vec<(String, bool)>,
}

static SENTENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[.!?…]+\s+").expect("sentence pattern"));
static OPENER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[^\p{L}]*((?:[\p{L}'’-]+\s+){0,2}[\p{L}'’-]+)\s*(?:,|—|–|\.\.\.|…)")
        .expect("opener pattern")
});
static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}'’]+(?:-[\p{L}'’]+)*").expect("word pattern"));
/// A speaker's label before a line, as cutscenes write it: `(-Ryne-)`.
static SPEAKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\(-[^)]*-\)").expect("speaker pattern"));

/// The text of a line without its speaker's label, on one line.
fn line_of(macro_text: &str) -> String {
    let plain = plain_text(macro_text).replace('\n', " ");
    SPEAKER.replace(&plain, "").into_owned()
}

impl Pair {
    /// A translated string's pair.
    #[must_use]
    pub fn of(path: &str, context: &str, source: &str, translation: &str) -> Self {
        let source_line = line_of(source).to_lowercase();
        let translation_line = line_of(translation);
        let mut openers = HashSet::new();
        let mut inner_words = Vec::new();
        for sentence in SENTENCE.split(&translation_line) {
            if let Some(found) = OPENER.captures(sentence) {
                openers.insert(found[1].to_lowercase());
            }
            for word in WORD.find_iter(sentence).skip(1) {
                let word = word.as_str();
                let capital = word.chars().next().is_some_and(char::is_uppercase);
                inner_words.push((word.to_lowercase(), capital));
            }
        }
        Self {
            path: path.to_owned(),
            context: context.to_owned(),
            source: source.to_owned(),
            translation: translation.to_owned(),
            source_words: WORD
                .find_iter(&source_line)
                .map(|word| word.as_str().to_owned())
                .collect(),
            openers,
            inner_words,
        }
    }
}

/// `part` of `whole` as a fraction; counts beyond `u32` are capped.
fn ratio(part: usize, whole: usize) -> f64 {
    let as_float = |count: usize| f64::from(u32::try_from(count).unwrap_or(u32::MAX));
    as_float(part) / as_float(whole.max(1))
}

/// The cues of an opener of `strings` strings, from how many of them each
/// source word comes with, most telling first: a word frequent in them and
/// far more frequent there than in the project's `translated` strings.
fn cues_of(
    strings: usize,
    words: &HashMap<&str, usize>,
    word_strings: &HashMap<&str, usize>,
    translated: usize,
) -> Vec<Cue> {
    let mut scored: Vec<(f64, &str, usize)> = words
        .iter()
        .filter_map(|(word, both)| {
            let share = ratio(*both, strings);
            let lift = share / ratio(word_strings[word], translated);
            (share >= MIN_CUE_SHARE && lift >= MIN_LIFT).then(|| (share * lift.ln(), *word, *both))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(MAX_CUES)
        .map(|(_, word, both)| Cue {
            word: word.to_owned(),
            strings: both,
        })
        .collect()
}

/// The openers of `pairs`, those of most strings first.
#[must_use]
pub fn openers_of(pairs: &[Pair]) -> Phrasing {
    let translated = pairs.len();
    let mut opener_strings: HashMap<&str, usize> = HashMap::new();
    let mut word_strings: HashMap<&str, usize> = HashMap::new();
    // Inside a sentence: how often a word is written with a capital, and how often at all.
    let mut capitals: HashMap<&str, (usize, usize)> = HashMap::new();
    for pair in pairs {
        for opener in &pair.openers {
            *opener_strings.entry(opener).or_default() += 1;
        }
        for word in &pair.source_words {
            *word_strings.entry(word).or_default() += 1;
        }
        for (word, capital) in &pair.inner_words {
            let seen = capitals.entry(word).or_default();
            seen.0 += usize::from(*capital);
            seen.1 += 1;
        }
    }
    let is_name = |phrase: &str| {
        phrase.split_whitespace().next().is_some_and(|first| {
            capitals
                .get(first)
                .is_some_and(|(capital, all)| *capital * 2 > *all)
        })
    };
    let mut chosen: Vec<(&str, usize)> = opener_strings
        .into_iter()
        .filter(|(phrase, strings)| *strings >= MIN_STRINGS && !is_name(phrase))
        .collect();
    chosen.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    chosen.truncate(MAX_OPENERS);

    let mut together: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    let wanted: HashSet<&str> = chosen.iter().map(|(phrase, _)| *phrase).collect();
    for pair in pairs {
        for opener in pair
            .openers
            .iter()
            .filter(|opener| wanted.contains(opener.as_str()))
        {
            let counts = together.entry(opener).or_default();
            for word in &pair.source_words {
                *counts.entry(word).or_default() += 1;
            }
        }
    }

    let mut openers: Vec<Opener> = chosen
        .iter()
        .map(|(phrase, strings)| Opener {
            phrase: (*phrase).to_owned(),
            strings: *strings,
            cues: together.get(phrase).map_or_else(Vec::new, |words| {
                cues_of(*strings, words, &word_strings, translated)
            }),
            unsupported: 0,
            examples: Vec::new(),
        })
        .collect();
    let index: HashMap<String, usize> = openers
        .iter()
        .enumerate()
        .map(|(at, opener)| (opener.phrase.clone(), at))
        .collect();
    for pair in pairs {
        for phrase in &pair.openers {
            let Some(&at) = index.get(phrase) else {
                continue;
            };
            let opener = &mut openers[at];
            if opener
                .cues
                .iter()
                .any(|cue| pair.source_words.contains(&cue.word))
            {
                continue;
            }
            opener.unsupported += 1;
            if opener.examples.len() < EXAMPLES {
                opener.examples.push(Example {
                    path: pair.path.clone(),
                    context: pair.context.clone(),
                    source: pair.source.clone(),
                    translation: pair.translation.clone(),
                });
            }
        }
    }
    Phrasing {
        translated,
        openers,
    }
}

/// The pairs of a file's translated strings that are not fuzzy.
fn read_pairs(root: &Path, path: &str) -> Result<Vec<Pair>, SearchError> {
    let full = root.join(PO_DIR).join(path);
    let text = std::fs::read_to_string(&full).map_err(|error| SearchError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    Ok(PoFile::parse(&text)
        .0
        .entries
        .iter()
        .filter(|entry| !entry.translation.is_empty() && !entry.fuzzy)
        .map(|entry| Pair::of(path, &entry.context, &entry.source, &entry.translation))
        .collect())
}

/// The openers of the translations of the project at `root`.
///
/// # Errors
///
/// Returns an error when `po/` or a file cannot be read.
pub fn phrasing(root: &Path) -> Result<Phrasing, SearchError> {
    let pairs: Vec<Pair> = read_each(root, read_pairs)?
        .into_iter()
        .flat_map(|(_, pairs)| pairs)
        .collect();
    Ok(openers_of(&pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(lines: &[(&str, &str)]) -> Vec<Pair> {
        lines
            .iter()
            .enumerate()
            .map(|(at, (source, translation))| {
                Pair::of("quest/a.po", &format!("q:{at}"), source, translation)
            })
            .collect()
    }

    #[test]
    fn an_opener_is_counted_with_the_source_words_behind_it() {
        let things = ["bridge", "gate", "road", "tower", "ship"];
        let seems: Vec<String> = (0..40)
            .map(|at| format!("It seems the {} broke.", things[at % things.len()]))
            .collect();
        // Sources that share no word: nothing of them is behind «Ну что,».
        let others: Vec<String> = (0..60).map(|at| "z".repeat(at + 1)).collect();
        let mut lines = Vec::new();
        for source in &seems {
            lines.push((source.as_str(), "Похоже, мост разрушен."));
        }
        for source in &others {
            lines.push((source.as_str(), "Ну что, пора идти."));
        }
        for _ in 0..400 {
            lines.push(("The gates are closed.", "Ворота закрыты."));
        }
        for _ in 0..20 {
            lines.push(("Apparently it rained.", "Похоже, шёл дождь."));
        }
        // A name the lines address: written with a capital inside sentences too.
        for _ in 0..60 {
            lines.push(("Alphinaud, wait.", "Альфино, постой. Где Альфино?"));
        }
        let found = openers_of(&pairs(&lines));
        let phrases: Vec<&str> = found.openers.iter().map(|o| o.phrase.as_str()).collect();
        assert_eq!(phrases, ["ну что", "похоже"], "{found:?}");
        let seems = &found.openers[1];
        assert_eq!(seems.strings, 60);
        let cues: Vec<&str> = seems.cues.iter().map(|cue| cue.word.as_str()).collect();
        assert!(
            cues.contains(&"seems") && cues.contains(&"apparently"),
            "{cues:?}"
        );
        assert_eq!(seems.unsupported, 0);
        let well = &found.openers[0];
        assert_eq!(well.strings, 60);
        assert_eq!(well.unsupported, 60, "{well:?}");
        assert!(well.cues.is_empty());
        assert!(well.examples.len() <= EXAMPLES);
    }

    #[test]
    fn macros_and_speakers_are_not_words() {
        let pair = Pair::of(
            "a.po",
            "a",
            "<i>Well</i>, I see.",
            "(-Ран'джит-)<i>Ну</i>, <if $gn4>ясно<else>понятно</if>.",
        );
        assert!(pair.openers.contains("ну"), "{:?}", pair.openers);
        assert!(pair.source_words.contains("well"));
        assert!(!pair.source_words.contains("i>"));
    }
}
