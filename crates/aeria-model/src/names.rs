//! The game is its own glossary: names of people, places, monsters, items,
//! actions, and statuses are strings of their sheets, and their
//! translations are how the project renders them everywhere. This finds the
//! translated names that occur in a text.

use std::collections::BTreeMap;
use std::path::Path;

use aeria_po::{Identity, PO_DIR, PoFile};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

/// Sheets whose strings are names, in the order a run translates them: the
/// world and who lives in it first, then what names use those words (an
/// action or item named after a place, a duty or quest named after both).
pub const NAME_SHEETS: [&str; 22] = [
    "PlaceName",
    "Town",
    "Race",
    "Tribe",
    "GuardianDeity",
    "ClassJob",
    "Status",
    "Action",
    "Trait",
    "Item",
    "EventItem",
    "Mount",
    "Companion",
    "Ornament",
    "Title",
    "ENpcResident",
    "BNpcName",
    "EObjName",
    "Fate",
    "InstanceContent",
    "ContentFinderCondition",
    "Quest",
];

/// The place in [`NAME_SHEETS`] of the sheet a file relative to `po/`
/// belongs to; `None` for a file of any other sheet.
#[must_use]
pub fn name_sheet_of(path: &str) -> Option<usize> {
    let sheet = path
        .strip_suffix(".po")
        .unwrap_or(path)
        .split('/')
        .next()?
        .trim_end_matches('~');
    NAME_SHEETS.iter().position(|name| *name == sheet)
}

/// Longest name taken from a name sheet; longer strings are descriptions.
const MAX_NAME: usize = 48;

/// Whether a source string reads as a name: short, one line, no macros, and
/// with a capital letter, since the game capitalizes names.
fn is_name(text: &str) -> bool {
    let length = text.chars().count();
    (3..=MAX_NAME).contains(&length)
        && !text.contains(['<', '\n', '.', '!', '?', ':'])
        && text.chars().any(char::is_uppercase)
}

/// Translated names of the project.
pub struct Names {
    known: Vec<(String, String)>,
    matcher: Option<AhoCorasick>,
    /// Names by their letters and digits in upper case, the form of a
    /// speaker label such as `ALPHINAUD`.
    labels: BTreeMap<String, usize>,
}

/// Each name once, with the translation it has most often; between equally
/// frequent translations, the one seen first. The same name in many rows of
/// the name sheets (a character in every place they stand) can be translated
/// several ways, and the most frequent is the project's usual one.
fn most_frequent(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut counts: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for (source, translation) in pairs {
        let variants = counts.entry(source).or_default();
        match variants.iter_mut().find(|(seen, _)| *seen == translation) {
            Some((_, count)) => *count += 1,
            None => variants.push((translation, 1)),
        }
    }
    counts
        .into_iter()
        .filter_map(|(source, variants)| {
            // `max_by_key` keeps the last of equals; reversed, the first seen.
            let (translation, _) = variants.into_iter().rev().max_by_key(|(_, count)| *count)?;
            Some((source, translation))
        })
        .collect()
}

/// A name's letters and digits in upper case.
fn label(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

impl Names {
    /// Reads the translated names from the name sheets' files under `po/`.
    /// A name translated more than one way keeps its most frequent
    /// translation (see [`most_frequent`]).
    #[must_use]
    pub fn load(root: &Path) -> Self {
        let mut found: Vec<(String, String)> = Vec::new();
        let paths = aeria_po::list(root).unwrap_or_default();
        for path in paths {
            if name_sheet_of(&path).is_none() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(root.join(PO_DIR).join(&path)) else {
                continue;
            };
            for entry in PoFile::parse(&text).0.entries {
                if entry.translation.is_empty() || entry.fuzzy || !is_name(&entry.source) {
                    continue;
                }
                if Identity::parse(&entry.context)
                    .is_ok_and(|identity| NAME_SHEETS.contains(&identity.sheet.as_str()))
                {
                    found.push((entry.source, entry.translation));
                }
            }
        }
        Self::new(most_frequent(found))
    }

    /// Names from pairs of source and translation.
    #[must_use]
    pub fn new(names: Vec<(String, String)>) -> Self {
        let matcher = (!names.is_empty())
            .then(|| {
                AhoCorasickBuilder::new()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(names.iter().map(|(source, _)| source.as_str()))
                    .ok()
            })
            .flatten();
        let mut labels = BTreeMap::new();
        for (index, (source, _)) in names.iter().enumerate() {
            labels.entry(label(source)).or_insert(index);
        }
        Self {
            known: names,
            matcher,
            labels,
        }
    }

    /// The name and translation a speaker label of a line stands for, such
    /// as `Alphinaud` for `ALPHINAUD`; `None` when no translated name has its
    /// letters, as for a label of a minor character.
    #[must_use]
    pub fn speaker(&self, speaker: &str) -> Option<(String, String)> {
        self.labels
            .get(&label(speaker))
            .map(|index| self.known[*index].clone())
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.known.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// The translated names that occur in `texts` as whole words, each once,
    /// at most `limit` of them.
    #[must_use]
    pub fn in_texts<'a>(
        &self,
        texts: impl IntoIterator<Item = &'a str>,
        limit: usize,
    ) -> Vec<(String, String)> {
        let Some(matcher) = &self.matcher else {
            return Vec::new();
        };
        let mut seen = Vec::new();
        for text in texts {
            for found in matcher.find_iter(text) {
                let before = text[..found.start()].chars().next_back();
                let after = text[found.end()..].chars().next();
                let boundary = |character: Option<char>| {
                    character.is_none_or(|character| !character.is_alphanumeric())
                };
                if !boundary(before) || !boundary(after) {
                    continue;
                }
                let index = found.pattern().as_usize();
                if !seen.contains(&index) {
                    seen.push(index);
                    if seen.len() == limit {
                        break;
                    }
                }
            }
        }
        seen.into_iter()
            .map(|index| self.known[index].clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_found_as_whole_words() {
        let names = Names::new(vec![
            ("Minfilia".to_owned(), "Минфилия".to_owned()),
            ("Limsa Lominsa".to_owned(), "Лимса Ломинса".to_owned()),
            ("Limsa".to_owned(), "Лимса".to_owned()),
            ("Fire".to_owned(), "Огонь".to_owned()),
        ]);
        let found = names.in_texts(
            ["Minfilia waits in Limsa Lominsa.", "Firearms and Minfilia."],
            10,
        );
        assert_eq!(
            found,
            vec![
                ("Minfilia".to_owned(), "Минфилия".to_owned()),
                ("Limsa Lominsa".to_owned(), "Лимса Ломинса".to_owned()),
            ]
        );
        assert_eq!(
            names.speaker("LIMSALOMINSA"),
            Some(("Limsa Lominsa".to_owned(), "Лимса Ломинса".to_owned()))
        );
        assert_eq!(names.speaker("FORTEMPSGUARD00054"), None);
        let pairs = |list: &[(&str, &str)]| {
            list.iter()
                .map(|(source, translation)| ((*source).to_owned(), (*translation).to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            most_frequent(pairs(&[
                ("Y'shtola", "Йштола"),
                ("Y'shtola", "Я'штола"),
                ("Y'shtola", "Я'штола"),
                ("Krile", "Крил"),
                ("Krile", "Крилэ"),
            ])),
            pairs(&[("Krile", "Крил"), ("Y'shtola", "Я'штола")])
        );
        assert_eq!(name_sheet_of("PlaceName.po"), Some(0));
        assert_eq!(name_sheet_of("Item/31000.po"), Some(9));
        assert_eq!(name_sheet_of("Quest~.po"), Some(NAME_SHEETS.len() - 1));
        assert_eq!(name_sheet_of("quest/001/X.po"), None);
        assert_eq!(name_sheet_of("ItemFood.po"), None);
        assert!(is_name("Mother Miounne"));
        assert!(!is_name("delivery moogle"));
        assert!(!is_name(
            "Restores 1,000 HP. Can only be used out of combat."
        ));
    }
}
