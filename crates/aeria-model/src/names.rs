//! The game is its own glossary: names of people, places, monsters, items,
//! actions, and statuses are strings of their sheets, and their
//! translations are how the project renders them everywhere. This finds the
//! translated names that occur in a text.

use std::collections::BTreeMap;
use std::fmt::Write as _;
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

/// A translated name of the game, with the string its translation comes
/// from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Name {
    pub source: String,
    pub translation: String,
    /// The `msgctxt` of the string the translation comes from; empty when
    /// unknown.
    pub context: String,
    /// The row's name when this string is another form of it, such as
    /// `The Walk` for `Walk`, the form a sentence uses without its article.
    pub full: Option<String>,
    /// Every name sheet with this name and translation, in the order of
    /// [`NAME_SHEETS`]: `Potion` is an item and an action.
    pub sheets: Vec<String>,
}

/// What the strings of a name sheet name, for the model.
const KINDS: [(&str, &str); 22] = [
    ("PlaceName", "a place"),
    ("Town", "a town"),
    ("Race", "a race"),
    ("Tribe", "a clan"),
    ("GuardianDeity", "a deity"),
    ("ClassJob", "a class or job"),
    ("Status", "a status effect"),
    ("Action", "an action"),
    ("Trait", "a trait"),
    ("Item", "an item"),
    ("EventItem", "a key item"),
    ("Mount", "a mount"),
    ("Companion", "a minion"),
    ("Ornament", "a fashion accessory"),
    ("Title", "a title"),
    ("ENpcResident", "a character"),
    ("BNpcName", "an enemy"),
    ("EObjName", "an object"),
    ("Fate", "a FATE"),
    ("InstanceContent", "a duty"),
    ("ContentFinderCondition", "a duty"),
    ("Quest", "a quest"),
];

impl Name {
    /// A name without a known origin.
    #[must_use]
    pub fn new(source: impl Into<String>, translation: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            translation: translation.into(),
            context: String::new(),
            full: None,
            sheets: Vec::new(),
        }
    }

    /// The sheet the name comes from; empty when unknown.
    #[must_use]
    pub fn sheet(&self) -> &str {
        self.context.split(':').next().unwrap_or_default()
    }

    /// What the name names, for the model: `the name of a place (sheet
    /// PlaceName), a form of "The Walk"`, or `the name of an item and an
    /// action (sheets Item, Action)`. `None` when the origin is unknown.
    #[must_use]
    pub fn origin(&self) -> Option<String> {
        let sheets: Vec<&str> = if self.sheets.is_empty() {
            vec![self.sheet()]
        } else {
            self.sheets.iter().map(String::as_str).collect()
        };
        let mut kinds: Vec<&str> = Vec::new();
        for sheet in &sheets {
            let kind = KINDS
                .iter()
                .find(|(name, _)| name == sheet)
                .map(|(_, kind)| *kind)?;
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        let kinds = match kinds.split_last() {
            Some((last, [])) => (*last).to_owned(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => return None,
        };
        let noun = if sheets.len() == 1 { "sheet" } else { "sheets" };
        let mut origin = format!("the name of {kinds} ({noun} {})", sheets.join(", "));
        if let Some(full) = &self.full {
            let _ = write!(origin, ", a form of \"{full}\"");
        }
        Some(origin)
    }
}

/// Translated names of the project.
pub struct Names {
    known: Vec<Name>,
    matcher: Option<AhoCorasick>,
    /// Names by their letters and digits in upper case, the form of a
    /// speaker label such as `ALPHINAUD`.
    labels: BTreeMap<String, usize>,
}

/// How well a string tells what its name names, best first: by the order
/// of [`NAME_SHEETS`], then a row's own name before another form of it.
/// `Minfilia` is a character (`ENpcResident`) before she is an enemy one
/// fights beside (`BNpcName`).
fn rank(name: &Name) -> (usize, bool) {
    let sheet = NAME_SHEETS
        .iter()
        .position(|sheet| *sheet == name.sheet())
        .unwrap_or(NAME_SHEETS.len());
    (sheet, name.full.is_some())
}

/// Each name once, with the translation it has most often and, of the
/// strings with it, the one that best tells what the name names (see
/// [`rank`]); between equally frequent translations, the one seen first.
/// The same name in many rows of the name sheets (a character in every
/// place they stand) can be translated several ways, and the most frequent
/// is the project's usual one.
fn most_frequent(names: Vec<Name>) -> Vec<Name> {
    let mut counts: BTreeMap<String, Vec<(Name, usize)>> = BTreeMap::new();
    for name in names {
        let variants = counts.entry(name.source.clone()).or_default();
        match variants
            .iter_mut()
            .find(|(seen, _)| seen.translation == name.translation)
        {
            Some((seen, count)) => {
                *count += 1;
                let mut sheets = std::mem::take(&mut seen.sheets);
                for sheet in &name.sheets {
                    if !sheets.contains(sheet) {
                        sheets.push(sheet.clone());
                    }
                }
                if rank(&name) < rank(seen) {
                    *seen = name;
                }
                sheets.sort_by_key(|sheet| {
                    NAME_SHEETS
                        .iter()
                        .position(|known| known == sheet)
                        .unwrap_or(NAME_SHEETS.len())
                });
                seen.sheets = sheets;
            }
            None => variants.push((name, 1)),
        }
    }
    counts
        .into_values()
        .filter_map(|variants| {
            // `max_by_key` keeps the last of equals; reversed, the first seen.
            let (name, _) = variants.into_iter().rev().max_by_key(|(_, count)| *count)?;
            Some(name)
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
        let mut found: Vec<Name> = Vec::new();
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
                let Ok(identity) = Identity::parse(&entry.context) else {
                    continue;
                };
                if !NAME_SHEETS.contains(&identity.sheet.as_str()) {
                    continue;
                }
                // Another form of the row's name, such as one without its
                // article, names the row by its first column.
                let full = (identity.column != 0)
                    .then(|| {
                        entry
                            .extracted
                            .iter()
                            .find_map(|line| line.strip_prefix("column 0: "))
                            .filter(|full| *full != entry.source)
                            .map(str::to_owned)
                    })
                    .flatten();
                found.push(Name {
                    source: entry.source,
                    translation: entry.translation,
                    sheets: vec![identity.sheet],
                    context: entry.context,
                    full,
                });
            }
        }
        Self::from_names(most_frequent(found))
    }

    /// Names from pairs of source and translation, without their origin.
    #[must_use]
    pub fn new(names: Vec<(String, String)>) -> Self {
        Self::from_names(
            names
                .into_iter()
                .map(|(source, translation)| Name::new(source, translation))
                .collect(),
        )
    }

    /// Names with their origins.
    #[must_use]
    pub fn from_names(names: Vec<Name>) -> Self {
        let matcher = (!names.is_empty())
            .then(|| {
                AhoCorasickBuilder::new()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(names.iter().map(|name| name.source.as_str()))
                    .ok()
            })
            .flatten();
        let mut labels = BTreeMap::new();
        for (index, name) in names.iter().enumerate() {
            labels.entry(label(&name.source)).or_insert(index);
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
        self.labels.get(&label(speaker)).map(|index| {
            let name = &self.known[*index];
            (name.source.clone(), name.translation.clone())
        })
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.known.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// The translated names that occur in `texts` as whole words with the
    /// same case, each once, at most `limit` of them. A name is found by its
    /// letters alone, so an ordinary word can be found as a name, as `Walk`
    /// (a form of the place `The Walk`) in `A Walk in the Park`.
    #[must_use]
    pub fn in_texts<'a>(
        &self,
        texts: impl IntoIterator<Item = &'a str>,
        limit: usize,
    ) -> Vec<Name> {
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
                Name::new("Minfilia", "Минфилия"),
                Name::new("Limsa Lominsa", "Лимса Ломинса"),
            ]
        );
        assert_eq!(
            names.speaker("LIMSALOMINSA"),
            Some(("Limsa Lominsa".to_owned(), "Лимса Ломинса".to_owned()))
        );
        assert_eq!(names.speaker("FORTEMPSGUARD00054"), None);
        let pairs = |list: &[(&str, &str)]| {
            list.iter()
                .map(|(source, translation)| Name::new(*source, *translation))
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
        let from_string = |source: &str, translation: &str, context: &str| Name {
            context: context.to_owned(),
            sheets: vec![context.split(':').next().unwrap_or_default().to_owned()],
            ..Name::new(source, translation)
        };
        assert_eq!(
            most_frequent(vec![
                from_string("Krile", "Крил", "BNpcName:9:0:0"),
                from_string("Krile", "Крил", "ENpcResident:1:0:0"),
                from_string("Krile", "Крил", "ENpcResident:2:0:0"),
            ]),
            vec![Name {
                sheets: vec!["ENpcResident".to_owned(), "BNpcName".to_owned()],
                ..from_string("Krile", "Крил", "ENpcResident:1:0:0")
            }],
            "a character before an enemy, then the first string, with every sheet"
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

    #[test]
    fn a_name_says_what_it_names() {
        let walk = Name {
            context: "PlaceName:1861:0:2".to_owned(),
            full: Some("The Walk".to_owned()),
            ..Name::new("Walk", "переход")
        };
        assert_eq!(walk.sheet(), "PlaceName");
        assert_eq!(
            walk.origin().as_deref(),
            Some("the name of a place (sheet PlaceName), a form of \"The Walk\"")
        );
        let alphinaud = Name {
            context: "ENpcResident:1005:0:0".to_owned(),
            ..Name::new("Alphinaud", "Альфино")
        };
        assert_eq!(
            alphinaud.origin().as_deref(),
            Some("the name of a character (sheet ENpcResident)")
        );
        let potion = Name {
            context: "Item:4551:0:0".to_owned(),
            sheets: vec!["Action".to_owned(), "Item".to_owned()],
            ..Name::new("Potion", "Зелье")
        };
        assert_eq!(
            potion.origin().as_deref(),
            Some("the name of an action and an item (sheets Action, Item)")
        );
        assert_eq!(Name::new("Fire", "Огонь").origin(), None);
    }
}
