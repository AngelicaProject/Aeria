//! What a request tells the model about one string, for the person who
//! translates it: the project's terms in its source, who says it, how long an interface label or a world object's name may
//! be, and whether its line varies with the player character's gender. A
//! request and the editor read these the same way, so the person sees what
//! the model is told: what a request comes to say about one string is read
//! here, or through functions this module shares with the request, and the
//! string guide shows it (principle 11 of `docs/product/principles.md`).

use aeria_knowledge::Glossary;
use aeria_po::Entry;
use aeria_po::length::{Budget, length_budget};

/// A term of the glossary that occurs in a string's source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HintTerm {
    pub term: String,
    pub translation: String,
    pub note: Option<String>,
    /// What the term names in this meaning.
    pub category: String,
    /// A person decided the term does not apply to this string.
    pub excepted: bool,
}

/// What one string's translation goes by besides its source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StringHints {
    pub terms: Vec<HintTerm>,
    /// The speaker label of a line (`ALPHINAUD`).
    pub speaker: Option<String>,
    /// The kind of a quest's text (`journal`, `objective`).
    pub kind: Option<String>,
    /// The longest an interface label's translation may show, or a world
    /// object's name may be.
    pub max_length: Option<Budget>,
    /// The texts whose line varies with the player character's gender:
    /// `source` and the languages of the context (`fr`, `de`).
    pub gendered: Vec<String>,
}

/// The hints of the string `entry`.
#[must_use]
pub fn hints(glossary: &Glossary, entry: &Entry) -> StringHints {
    let line = |prefix: &str| {
        entry
            .extracted
            .iter()
            .find_map(|line| line.strip_prefix(prefix))
            .map(str::to_owned)
    };
    let found = glossary.matches(&entry.source);
    let applying = glossary.matches_except(&entry.source, &entry.term_exceptions);
    StringHints {
        terms: found
            .into_iter()
            .map(|term| HintTerm {
                term: term.term.clone(),
                translation: term.translation.clone(),
                note: term.full_note(),
                category: term.category.clone(),
                excepted: !applying.iter().any(|entry| entry.term == term.term),
            })
            .collect(),
        speaker: line("speaker: "),
        kind: line("kind: "),
        max_length: length_budget(entry),
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
    fn a_line_has_its_terms_speaker_and_gender() {
        let glossary = Glossary::new(vec![term("Scions", "Потомки"), term("aether", "эфир")]);
        let found = hints(
            &glossary,
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
            found
                .terms
                .iter()
                .map(|term| (term.term.as_str(), term.excepted))
                .collect::<Vec<_>>(),
            vec![("Scions", false), ("aether", true)]
        );
        assert_eq!(found.speaker.as_deref(), Some("ALPHINAUD"));
        assert_eq!(found.gendered, vec!["source", "fr"]);
        assert_eq!(found.max_length, None);
    }

    #[test]
    fn an_interface_label_has_its_length() {
        let found = hints(
            &Glossary::default(),
            &Entry {
                context: "Addon:1:0:0".to_owned(),
                ..entry(
                    "Direct Hit",
                    &["de: Direkter Treffer", "fr: Coup direct"],
                    &[],
                )
            },
        );
        assert_eq!(
            found.max_length,
            Some(Budget {
                max: "Direkter Treffer".len(),
                unit: aeria_po::length::Unit::Characters,
            })
        );
        assert_eq!(found.speaker, None);
    }
}
