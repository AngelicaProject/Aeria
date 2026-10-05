//! What a request tells the model about one string, for the person who
//! translates it: the game's names and the project's terms in its source,
//! who says it, how long an interface label may be, and whether its line
//! varies with the player character's gender. A request and the editor read
//! these the same way, so the person sees what the model is told.

use aeria_knowledge::Glossary;
use aeria_po::Entry;

use crate::fit;
use crate::names::Names;

/// Most names of one string listed; a request of many strings has more.
const NAMES: usize = 40;

/// A term of the glossary that occurs in a string's source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HintTerm {
    pub term: String,
    pub translation: String,
    pub note: Option<String>,
    pub never: Vec<String>,
    /// A person decided the term does not apply to this string.
    pub excepted: bool,
}

/// What one string's translation goes by besides its source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StringHints {
    /// The game's names in the source with the project's translations.
    pub names: Vec<(String, String)>,
    pub terms: Vec<HintTerm>,
    /// The speaker label of a line (`ALPHINAUD`), with the name and
    /// translation it stands for when the project translates that name.
    pub speaker: Option<(String, Option<(String, String)>)>,
    /// The kind of a quest's text (`journal`, `objective`).
    pub kind: Option<String>,
    /// The most characters an interface label's translation may show.
    pub max_length: Option<usize>,
    /// The texts whose line varies with the player character's gender:
    /// `source` and the languages of the context (`fr`, `de`).
    pub gendered: Vec<String>,
}

/// The hints of the string `entry` of the file `path`, relative to `po/`.
#[must_use]
pub fn hints(names: &Names, glossary: &Glossary, path: &str, entry: &Entry) -> StringHints {
    let line = |prefix: &str| {
        entry
            .extracted
            .iter()
            .find_map(|line| line.strip_prefix(prefix))
            .map(str::to_owned)
    };
    let excepted = glossary.matches(&entry.source);
    let applying = glossary.matches_except(&entry.source, &entry.term_exceptions);
    StringHints {
        names: names.in_texts([entry.source.as_str()], NAMES),
        terms: excepted
            .into_iter()
            .map(|term| HintTerm {
                term: term.term.clone(),
                translation: term.translation.clone(),
                note: term.note.clone(),
                never: term.forbidden.clone(),
                excepted: !applying.iter().any(|entry| entry.term == term.term),
            })
            .collect(),
        speaker: line("speaker: ").map(|label| {
            let name = names.speaker(&label);
            (label, name)
        }),
        kind: line("kind: "),
        max_length: fit::length_budget(path, &entry.source, &entry.extracted),
        gendered: gendered(&entry.source, &entry.extracted)
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

/// The texts of a string whose line varies with the player character's
/// gender: `source`, and the languages of its context (`fr: …`) with a
/// condition on `$gn4`.
#[must_use]
pub fn gendered<'a>(source: &str, context: &'a [String]) -> Vec<&'a str> {
    let mut texts = Vec::new();
    if source.contains("$gn4") {
        texts.push("source");
    }
    for line in context {
        if let Some((language, text)) = line.split_once(": ")
            && language.len() == 2
            && language.bytes().all(|byte| byte.is_ascii_lowercase())
            && text.contains("$gn4")
        {
            texts.push(language);
        }
    }
    texts
}

#[cfg(test)]
mod tests {
    use aeria_knowledge::GlossaryEntry;

    use super::*;

    fn entry(source: &str, extracted: &[&str], exceptions: &[&str]) -> Entry {
        Entry {
            source: source.to_owned(),
            extracted: extracted.iter().map(|line| (*line).to_owned()).collect(),
            term_exceptions: exceptions.iter().map(|term| (*term).to_owned()).collect(),
            ..Entry::default()
        }
    }

    fn term(term: &str, translation: &str) -> GlossaryEntry {
        GlossaryEntry {
            term: term.to_owned(),
            translation: translation.to_owned(),
            ..GlossaryEntry::default()
        }
    }

    #[test]
    fn a_line_has_its_names_terms_speaker_and_gender() {
        let names = Names::new(vec![
            ("Minfilia".to_owned(), "Минфилия".to_owned()),
            ("Alphinaud".to_owned(), "Альфино".to_owned()),
        ]);
        let glossary = Glossary::new(vec![term("Scions", "Потомки"), term("aether", "эфир")]);
        let found = hints(
            &names,
            &glossary,
            "quest/000/X.po",
            &entry(
                "The Scions ask Minfilia about the aether, <if $gn4>lass<else>lad</if>.",
                &[
                    "fr: <if $gn4>elle<else>il</if>",
                    "de: Er",
                    "speaker: ALPHINAUD",
                ],
                &["aether"],
            ),
        );
        assert_eq!(
            found.names,
            vec![("Minfilia".to_owned(), "Минфилия".to_owned())]
        );
        assert_eq!(
            found
                .terms
                .iter()
                .map(|term| (term.term.as_str(), term.excepted))
                .collect::<Vec<_>>(),
            vec![("Scions", false), ("aether", true)]
        );
        assert_eq!(
            found.speaker,
            Some((
                "ALPHINAUD".to_owned(),
                Some(("Alphinaud".to_owned(), "Альфино".to_owned()))
            ))
        );
        assert_eq!(found.gendered, vec!["source", "fr"]);
        assert_eq!(found.max_length, None);
    }

    #[test]
    fn an_interface_label_has_its_length() {
        let found = hints(
            &Names::new(Vec::new()),
            &Glossary::default(),
            "Addon.po",
            &entry(
                "Direct Hit",
                &["de: Direkter Treffer", "fr: Coup direct"],
                &[],
            ),
        );
        assert_eq!(found.max_length, Some("Direkter Treffer".len()));
        assert_eq!(found.speaker, None);
    }
}
