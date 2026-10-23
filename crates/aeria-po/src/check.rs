//! The checks of a translation and of a file. Aeria runs them when a
//! translation is saved or machine-translated, and over the files of a
//! project; a translation with a problem is never saved.

use std::collections::BTreeSet;

use aeria_knowledge::Knowledge;

use crate::identity::Identity;
use crate::length::{Unit, length_budget};
use crate::po::{Entry, PoFile};

/// What the checks of a translation found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Verdict {
    /// What must be fixed before the translation is saved.
    pub problems: Vec<String>,
    /// What may be wrong; it does not stop the translation.
    pub advice: Vec<String>,
    /// The problems, then the advice, as data an interface can word in its
    /// own language; `problems` and `advice` are their messages.
    pub issues: Vec<Issue>,
}

impl Verdict {
    fn of(issues: Vec<Issue>) -> Self {
        let (problems, advice): (Vec<&Issue>, Vec<&Issue>) =
            issues.iter().partition(|issue| issue.is_problem());
        Self {
            problems: problems.iter().map(ToString::to_string).collect(),
            advice: advice.iter().map(ToString::to_string).collect(),
            issues,
        }
    }
}

/// One finding of the checks of a translation: what is broken whatever the
/// translation says. Whether it renders its terms and follows the style is
/// not a check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Issue {
    /// A line break the source does not have.
    LineBreak,
    /// The structure policy refused the macros (`aeria_se`); the message is
    /// written for the model that produced the translation.
    Structure(String),
    /// Advice: a term exception names no term of the source: the term left
    /// the glossary, or the source changed.
    StaleTermException(String),
    /// Advice: the name of a world object has more bytes than the game
    /// shows; the game cuts it (see [`crate::length`]). Machine translation
    /// is refused for it.
    NameTooLong { length: usize, max: usize },
}

impl Issue {
    /// What groups issues for a summary: the kind.
    #[must_use]
    pub fn group(&self) -> String {
        match self {
            Self::LineBreak => "lineBreak",
            Self::Structure(_) => "structure",
            Self::StaleTermException(_) => "staleTermException",
            Self::NameTooLong { .. } => "nameTooLong",
        }
        .to_owned()
    }

    /// Whether the issue is of `group`, or of its kind when `group` names
    /// the kind alone.
    #[must_use]
    pub fn is_of(&self, group: &str) -> bool {
        let own = self.group();
        own == group || own.split_once(':').is_some_and(|(kind, _)| kind == group)
    }

    /// A problem stops a translation from being saved and exported; the rest
    /// is advice.
    #[must_use]
    pub fn is_problem(&self) -> bool {
        matches!(self, Self::LineBreak | Self::Structure(_))
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LineBreak => f.write_str(
                "the translation has a line break the source does not; the game breaks lines with <br>",
            ),
            Self::Structure(message) => f.write_str(message),
            Self::StaleTermException(term) => write!(
                f,
                "the string has an exception for the term {term:?}, which its source does not have; remove the exception"
            ),
            Self::NameTooLong { length, max } => write!(
                f,
                "the name is {length} bytes and the game shows at most {max} over the character or object"
            ),
        }
    }
}

/// Checks `text` as the translation of `entry` against its source, its `#.`
/// lines (with the other client languages), the terms a person decided do
/// not apply to it, and the length the game shows of a name.
#[must_use]
pub fn check_translation(
    knowledge: &Knowledge,
    _target_language: &str,
    entry: &Entry,
    text: &str,
) -> Verdict {
    let source = entry.source.as_str();
    let mut issues = Vec::new();
    if text.contains('\n') && !source.contains('\n') {
        issues.push(Issue::LineBreak);
    }
    // The same string in the other client languages: game data they use
    // in place of the source's may stand in the translation.
    let localizations: Vec<&str> = entry
        .extracted
        .iter()
        .filter_map(|line| {
            ["ja: ", "de: ", "fr: "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
        })
        .collect();
    if let Err(errors) = aeria_se::check_assisted_structure_with(source, text, &localizations) {
        issues.extend(
            errors
                .into_iter()
                .map(|error| Issue::Structure(error.message)),
        );
    }
    issues.extend(
        knowledge
            .terms
            .stale_exceptions(source, &entry.term_exceptions)
            .into_iter()
            .map(|term| Issue::StaleTermException(term.to_owned())),
    );
    if let Some(budget) = length_budget(entry)
        && budget.unit == Unit::Bytes
    {
        let length = budget.length_of(text);
        if length > budget.max {
            issues.push(Issue::NameTooLong {
                length,
                max: budget.max,
            });
        }
    }
    Verdict::of(issues)
}

/// One finding of a file check, at a line of the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub line: usize,
    pub message: String,
    /// Advice rather than a problem.
    pub advice: bool,
}

/// Checks the entries of a file without the game: identities, duplicates,
/// and every translation against its `msgid`.
#[must_use]
pub fn check_file(file: &PoFile, knowledge: &Knowledge, target_language: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in &file.entries {
        if let Err(message) = Identity::parse(&entry.context) {
            findings.push(Finding {
                line: entry.line,
                message,
                advice: false,
            });
            continue;
        }
        if !seen.insert(entry.context.as_str()) {
            findings.push(Finding {
                line: entry.line,
                message: format!("{} appears twice in this file", entry.context),
                advice: false,
            });
            continue;
        }
        if entry.translation.is_empty() {
            continue;
        }
        let verdict = check_translation(knowledge, target_language, entry, &entry.translation);
        findings.extend(verdict.problems.into_iter().map(|message| Finding {
            line: entry.line,
            message,
            advice: false,
        }));
        findings.extend(verdict.advice.into_iter().map(|message| Finding {
            line: entry.line,
            message,
            advice: true,
        }));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(
        knowledge: &Knowledge,
        target_language: &str,
        source: &str,
        text: &str,
        extracted: &[String],
    ) -> Verdict {
        let entry = Entry {
            source: source.to_owned(),
            extracted: extracted.to_vec(),
            ..Entry::default()
        };
        check_translation(knowledge, target_language, &entry, text)
    }

    #[test]
    fn game_data_of_the_other_languages_of_the_string_may_stand_in() {
        let source = "<title-case><sheet ClassJob $n1 30></title-case> (Lv. <num $n2>)";
        let text = "<capitalize><sheet ClassJob $n1 0></capitalize> (ур. <num $n2>)";
        let extracted = [
            "ja: <sheet ClassJob $n1 30>   Lv.<num $n2>".to_owned(),
            "de: <sheet ClassJob $n1 0> St. <num $n2>".to_owned(),
            "fr: <capitalize><sheet ClassJob $n1 0></capitalize> nv <num $n2>".to_owned(),
        ];
        let knowledge = Knowledge::default();
        assert!(
            check(&knowledge, "ru", source, text, &extracted)
                .problems
                .is_empty()
        );
        assert!(
            !check(&knowledge, "ru", source, text, &[])
                .problems
                .is_empty()
        );
    }

    #[test]
    fn a_line_break_the_source_does_not_have_is_a_problem() {
        let knowledge = Knowledge::default();
        assert!(
            check(&knowledge, "ru", "Hello.", "Привет.", &[])
                .problems
                .is_empty()
        );
        // What a translation says is not checked.
        for text in ["Ты готов(а).", "Я в в городе.", "Это является ошибкой."]
        {
            assert!(
                check(&knowledge, "ru", "You are ready.", text, &[])
                    .issues
                    .is_empty(),
                "{text}"
            );
        }
        let verdict = check(&knowledge, "ru", "Hello.", "При\nвет.", &[]);
        assert_eq!(verdict.problems.len(), 1, "{verdict:?}");
    }

    #[test]
    fn a_name_longer_than_the_game_shows_is_advice() {
        let knowledge = Knowledge::default();
        let entry = |context: &str, source: &str, extracted: &[&str]| Entry {
            context: context.to_owned(),
            source: source.to_owned(),
            extracted: extracted.iter().map(|line| (*line).to_owned()).collect(),
            ..Entry::default()
        };
        let aide = entry(
            "ENpcResident:1019070:0:0",
            "East Aldenard Trading Company aide",
            &[],
        );
        let verdict = check_translation(
            &knowledge,
            "ru",
            &aide,
            "служащий торговой компании «Восточный Альденард»",
        );
        assert!(verdict.problems.is_empty(), "{verdict:?}");
        assert_eq!(
            verdict.issues,
            vec![Issue::NameTooLong {
                length: 92,
                max: 63
            }]
        );
        assert!(
            check_translation(&knowledge, "ru", &aide, "служащий ТК «Восточный Альденард»")
                .issues
                .is_empty()
        );
        // An interface label's length is guidance for machine translation.
        let loot = entry("Addon:1:0:0", "Loot", &["de: Beutegut", "fr: Butin"]);
        assert!(
            check_translation(&knowledge, "ru", &loot, "Военные трофеи")
                .issues
                .is_empty()
        );
    }

    #[test]
    fn a_file_reports_duplicates_and_bad_identities() {
        let (file, problems) = PoFile::parse(
            "msgctxt \"Addon:1:0:0\"\nmsgid \"OK\"\nmsgstr \"ОК\"\n\nmsgctxt \"Addon:1:0:0\"\nmsgid \"OK\"\nmsgstr \"\"\n\nmsgctxt \"Addon\"\nmsgid \"x\"\nmsgstr \"\"\n",
        );
        assert!(problems.is_empty());
        let findings = check_file(&file, &Knowledge::default(), "ru");
        assert_eq!(findings.len(), 2, "{findings:?}");
    }
}
