//! Terminology candidates: short names from the game's data sheets that
//! recur inside other strings.
//!
//! A *name* is a translatable cell of a data sheet (a sheet name without
//! `/`, so not quest, cutscene, or other script text) whose whole text is a
//! short name without macros or sentence punctuation, such as a place, an
//! NPC, an item, or an action. A name is a candidate when its exact text
//! occurs, as whole words and with the same case, in other translatable
//! strings. Everything here is deterministic; deciding which candidates are
//! terminology is left to the translator or Angelica.

use std::collections::HashMap;

use aho_corasick::{AhoCorasick, MatchKind};

use crate::index::{SearchError, SourceHit, Tokenizer};

/// Most words in a name.
pub const MAX_TERM_WORDS: usize = 5;
/// Most characters in a name.
pub const MAX_TERM_CHARS: usize = 48;
/// Locations kept for each candidate.
pub const MAX_TERM_LOCATIONS: usize = 3;

/// A name that recurs in other strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TermCandidate {
    /// The name as the source writes it.
    pub term: String,
    /// Up to [`MAX_TERM_LOCATIONS`] cells whose whole text is the name.
    pub locations: Vec<SourceHit>,
    /// Cells whose whole text is the name.
    pub names: usize,
    /// Other strings that contain the name.
    pub strings: usize,
    /// Occurrences in those strings.
    pub occurrences: usize,
}

/// Punctuation allowed inside a name besides letters, digits, and spaces.
fn name_punctuation(character: char) -> bool {
    matches!(character, '\'' | '’' | '-' | '&' | '・' | 'ー')
}

/// Whether a cell's text is a name: see the module documentation.
fn is_name(sheet: &str, source: &str, plain: &str, tokenizer: Tokenizer) -> bool {
    if sheet.contains('/') || source != plain || plain.trim() != plain {
        return false;
    }
    let characters = plain.chars().count();
    if !(2..=MAX_TERM_CHARS).contains(&characters)
        || plain.split_whitespace().count() > MAX_TERM_WORDS
        || !plain.chars().any(char::is_alphabetic)
        || !plain
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || name_punctuation(c))
    {
        return false;
    }
    match tokenizer {
        Tokenizer::Words => plain.chars().next().is_some_and(char::is_uppercase),
        Tokenizer::Trigram => true,
    }
}

/// Whether a match at `start..end` of `text` stands as whole words.
fn whole_words(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
}

/// Whether `start` begins a sentence or a line of `text`, where any word
/// is capitalized.
fn starts_sentence(text: &str, start: usize) -> bool {
    let before =
        text[..start].trim_end_matches(|c: char| c.is_whitespace() || "\"'“‘«(".contains(c));
    before.is_empty()
        || before.ends_with(['.', '!', '?', '…', ':', '—'])
        || text[before.len()..start].contains('\n')
}

/// Collects names, then counts them in every string.
pub(crate) struct TermFinder {
    tokenizer: Tokenizer,
    terms: Vec<String>,
    by_text: HashMap<String, usize>,
    locations: Vec<Vec<SourceHit>>,
    names: Vec<usize>,
}

impl TermFinder {
    pub(crate) fn new(tokenizer: Tokenizer) -> Self {
        Self {
            tokenizer,
            terms: Vec::new(),
            by_text: HashMap::new(),
            locations: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Records a cell when its text is a name.
    pub(crate) fn add(&mut self, hit: SourceHit, plain: &str) {
        if !is_name(&hit.sheet, &hit.source, plain, self.tokenizer) {
            return;
        }
        let index = *self.by_text.entry(plain.to_owned()).or_insert_with(|| {
            self.terms.push(plain.to_owned());
            self.locations.push(Vec::new());
            self.names.push(0);
            self.terms.len() - 1
        });
        self.names[index] += 1;
        if self.locations[index].len() < MAX_TERM_LOCATIONS {
            self.locations[index].push(hit);
        }
    }

    pub(crate) fn counter(self) -> Result<TermCounter, SearchError> {
        let automaton = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&self.terms)
            .map_err(|error| SearchError::Io(std::io::Error::other(error)))?;
        let count = self.terms.len();
        Ok(TermCounter {
            finder: self,
            automaton,
            strings: vec![0; count],
            occurrences: vec![0; count],
            seen: vec![usize::MAX; count],
            text_number: 0,
        })
    }
}

pub(crate) struct TermCounter {
    finder: TermFinder,
    automaton: AhoCorasick,
    strings: Vec<usize>,
    occurrences: Vec<usize>,
    /// The number of the last text each name was counted in.
    seen: Vec<usize>,
    text_number: usize,
}

impl TermCounter {
    /// Counts the names in one string's plain text. A string that is
    /// exactly a name does not count for it.
    pub(crate) fn count(&mut self, plain: &str) {
        self.text_number += 1;
        let whole = plain.trim();
        for found in self.automaton.find_iter(plain) {
            let (start, end) = (found.start(), found.end());
            let index = found.pattern().as_usize();
            if whole == self.finder.terms[index] {
                continue;
            }
            if self.finder.tokenizer == Tokenizer::Words {
                if !whole_words(plain, start, end) {
                    continue;
                }
                if starts_sentence(plain, start) {
                    continue;
                }
            }
            self.occurrences[index] += 1;
            if self.seen[index] != self.text_number {
                self.seen[index] = self.text_number;
                self.strings[index] += 1;
            }
        }
    }

    /// Names found in at least `min_strings` other strings, those in the
    /// most strings first.
    pub(crate) fn finish(self, min_strings: usize) -> Vec<TermCandidate> {
        let finder = self.finder;
        let mut candidates: Vec<TermCandidate> = finder
            .terms
            .into_iter()
            .zip(finder.locations)
            .zip(finder.names)
            .zip(self.strings.into_iter().zip(self.occurrences))
            .filter(|(_, (strings, _))| *strings >= min_strings.max(1))
            .map(
                |(((term, locations), names), (strings, occurrences))| TermCandidate {
                    term,
                    locations,
                    names,
                    strings,
                    occurrences,
                },
            )
            .collect();
        candidates.sort_by(|left, right| {
            right
                .strings
                .cmp(&left.strings)
                .then(right.occurrences.cmp(&left.occurrences))
                .then_with(|| left.term.cmp(&right.term))
        });
        candidates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(sheet: &str, row: u32, text: &str) -> SourceHit {
        SourceHit {
            sheet: sheet.to_owned(),
            row,
            subrow: 0,
            column: 0,
            source: text.to_owned(),
        }
    }

    fn candidates(names: &[(&str, &str)], texts: &[&str]) -> Vec<TermCandidate> {
        let mut finder = TermFinder::new(Tokenizer::Words);
        for (row, (sheet, text)) in names.iter().enumerate() {
            finder.add(hit(sheet, u32::try_from(row).expect("row"), text), text);
        }
        let mut counter = finder.counter().expect("counter");
        for (_, text) in names {
            counter.count(text);
        }
        for text in texts {
            counter.count(text);
        }
        counter.finish(1)
    }

    #[test]
    fn names_are_short_data_texts_without_sentences() {
        let words = Tokenizer::Words;
        assert!(is_name(
            "PlaceName",
            "Limsa Lominsa",
            "Limsa Lominsa",
            words
        ));
        assert!(is_name("ENpcResident", "Y'shtola", "Y'shtola", words));
        assert!(!is_name(
            "quest/001/X",
            "Limsa Lominsa",
            "Limsa Lominsa",
            words
        ));
        assert!(!is_name("Addon", "Go on.", "Go on.", words));
        assert!(!is_name("Item", "lowercase", "lowercase", words));
        assert!(!is_name("Item", "<i>Fire</i>", "Fire", words));
        assert!(!is_name("Item", "A B C D E F", "A B C D E F", words));
        assert!(is_name(
            "PlaceName",
            "リムサ・ロミンサ",
            "リムサ・ロミンサ",
            Tokenizer::Trigram
        ));
    }

    #[test]
    fn names_are_counted_as_whole_words_in_other_strings() {
        let found = candidates(
            &[
                ("PlaceName", "Limsa Lominsa"),
                ("PlaceName", "Limsa Lominsa"),
                ("ENpcResident", "Merlwyb"),
                ("Item", "Fire"),
                ("Item", "Fire Shard"),
                ("Item", "Lominsan"),
            ],
            &[
                "Welcome to Limsa Lominsa, where Merlwyb rules.",
                "Merlwyb awaits in Limsa Lominsa. Limsa Lominsa never sleeps.",
                "Bring a Fire Shard. Fire burns.",
                "The Lominsans cheer.",
                "Ask for Merlwyb's fleet.",
                "Sail on\nMerlwyb stays ashore.",
                "Back to Limsa Lominsa and Limsa Lominsa again",
            ],
        );
        let term = |name: &str| found.iter().find(|candidate| candidate.term == name);
        let limsa = term("Limsa Lominsa").expect("Limsa Lominsa");
        assert_eq!(
            (limsa.names, limsa.strings, limsa.occurrences),
            (2, 3, 4),
            "sentence starts do not count"
        );
        assert_eq!(limsa.locations.len(), 2);
        let merlwyb = term("Merlwyb").expect("Merlwyb");
        assert_eq!(
            (merlwyb.strings, merlwyb.occurrences),
            (2, 2),
            "not at a sentence start"
        );
        assert_eq!(term("Fire Shard").expect("shard").strings, 1);
        assert!(
            term("Fire").is_none(),
            "the longest name wins and sentence starts do not count"
        );
        assert!(term("Lominsan").is_none(), "only whole words count");
        assert_eq!(found[0].term, "Limsa Lominsa");
    }
}
