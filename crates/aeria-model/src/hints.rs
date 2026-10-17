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
    /// A person, or a correction run's model, decided the term does not
    /// apply to this string.
    pub excepted: bool,
    /// The decision is the model's: its model exception holds and the
    /// person made none.
    pub by_model: bool,
    /// The words a correction run's model decided render the term in this
    /// string's translation, while the decisions hold and the term applies.
    pub rendered: Vec<String>,
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
    let excepted = glossary.matches(&entry.source);
    let exceptions = aeria_po::applied_exceptions(entry, glossary);
    let applying = glossary.matches_except(&entry.source, &exceptions);
    let by_person = glossary.matches_except(&entry.source, &entry.term_exceptions);
    StringHints {
        terms: excepted
            .into_iter()
            .map(|term| {
                let excepted = !applying.iter().any(|entry| entry.term == term.term);
                HintTerm {
                    term: term.term.clone(),
                    translation: term.translation.clone(),
                    note: term.full_note(),
                    category: term.category.clone(),
                    excepted,
                    by_model: excepted && by_person.iter().any(|entry| entry.term == term.term),
                    rendered: entry
                        .model_renderings
                        .iter()
                        .filter(|rendering| {
                            !excepted
                                && rendering.term.to_lowercase() == term.term.to_lowercase()
                                && rendering.holds(glossary)
                        })
                        .map(|rendering| rendering.word.clone())
                        .collect(),
                }
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
    fn a_model_decision_shows_as_the_models_while_it_holds() {
        let mount = term("mount", "маунт");
        let glossary = Glossary::new(vec![mount.clone()]);
        let decided = Entry {
            model_exceptions: vec![aeria_po::ModelException::of(&mount)],
            ..entry("Mount an attack.", &[], &[])
        };
        let found = hints(&glossary, &decided);
        assert!(found.terms[0].excepted && found.terms[0].by_model);
        // A word the model decided renders the term shows while the term
        // applies.
        let rendered = Entry {
            model_renderings: vec![aeria_po::ModelRendering::of(&mount, "маунтовый")],
            ..entry("Mount an attack.", &[], &[])
        };
        let found = hints(&glossary, &rendered);
        assert_eq!(found.terms[0].rendered, ["маунтовый"]);
        assert!(!found.terms[0].excepted);
        let excepted = Entry {
            term_exceptions: vec!["mount".to_owned()],
            ..rendered
        };
        let found = hints(&glossary, &excepted);
        assert!(found.terms[0].rendered.is_empty());
        // A person's exception is the person's.
        let both = Entry {
            term_exceptions: vec!["mount".to_owned()],
            ..decided.clone()
        };
        let found = hints(&glossary, &both);
        assert!(found.terms[0].excepted && !found.terms[0].by_model);
        // Another translation of the term asks for a new decision.
        let changed = Glossary::new(vec![term("mount", "ездовое животное")]);
        let found = hints(&changed, &decided);
        assert!(!found.terms[0].excepted && !found.terms[0].by_model);
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
