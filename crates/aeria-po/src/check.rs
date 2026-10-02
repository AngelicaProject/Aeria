//! The checks of a translation and of a file. Aeria runs them when a
//! translation is saved or machine-translated, and over the files of a
//! project; a translation with a problem is never saved.

use std::collections::BTreeSet;

use aeria_knowledge::Knowledge;
use aeria_knowledge::rules::machine_phrasing;

use crate::identity::Identity;
use crate::po::PoFile;

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

/// One finding of the checks of a translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Issue {
    /// A line break the source does not have.
    LineBreak,
    /// The structure policy refused the macros (`aeria_se`); the message is
    /// written for the model that produced the translation.
    Structure(String),
    /// A forbidden variant of a term of the source.
    ForbiddenTerm {
        term: String,
        translation: String,
        variant: String,
    },
    /// A stress or other combining mark the source does not have.
    Mark(char),
    /// A word that mixes alphabets, or has letters of another writing system.
    MixedAlphabets(String),
    /// A Russian form that writes both genders at once, such as `(а)`.
    BothGenders(String),
    /// Advice: the translation of a term of the source does not seem used.
    TermNotUsed { term: String, translation: String },
    /// Advice: the source varies with the player character's gender and the
    /// translation does not.
    GenderNotVaried,
    /// Advice: the French or German line varies with the player character's
    /// gender and the translation does not.
    GenderInOtherLanguages,
    /// Advice: phrasing that reads machine-written.
    MachinePhrasing(Vec<String>),
    /// Advice: a term exception names no term of the source: the term left
    /// the glossary, or the source changed.
    StaleTermException(String),
}

impl Issue {
    /// What groups issues for a summary: the kind, with the term for term
    /// issues (`termNotUsed:pugilist`).
    #[must_use]
    pub fn group(&self) -> String {
        match self {
            Self::LineBreak => "lineBreak".to_owned(),
            Self::Structure(_) => "structure".to_owned(),
            Self::ForbiddenTerm { term, .. } => format!("forbiddenTerm:{}", term.to_lowercase()),
            Self::Mark(_) => "mark".to_owned(),
            Self::MixedAlphabets(_) => "mixedAlphabets".to_owned(),
            Self::BothGenders(_) => "bothGenders".to_owned(),
            Self::TermNotUsed { term, .. } => format!("termNotUsed:{}", term.to_lowercase()),
            Self::GenderNotVaried => "genderNotVaried".to_owned(),
            Self::GenderInOtherLanguages => "genderInOtherLanguages".to_owned(),
            Self::MachinePhrasing(_) => "machinePhrasing".to_owned(),
            Self::StaleTermException(_) => "staleTermException".to_owned(),
        }
    }

    /// A problem stops a translation from being saved and exported; the rest
    /// is advice.
    #[must_use]
    pub fn is_problem(&self) -> bool {
        !matches!(
            self,
            Self::TermNotUsed { .. }
                | Self::GenderNotVaried
                | Self::GenderInOtherLanguages
                | Self::MachinePhrasing(_)
                | Self::StaleTermException(_)
        )
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LineBreak => f.write_str(
                "the translation has a line break the source does not; the game breaks lines with <br>",
            ),
            Self::Structure(message) => f.write_str(message),
            Self::ForbiddenTerm {
                term,
                translation,
                variant,
            } => write!(f, "the terms forbid {variant:?} for {term:?}; use {translation:?}"),
            Self::Mark(mark) => write!(
                f,
                "the translation has the mark U+{:04X} (a stress or accent); write the word without it",
                u32::from(*mark)
            ),
            Self::MixedAlphabets(word) => write!(
                f,
                "«{word}» mixes letters of different alphabets; write it in Cyrillic only"
            ),
            Self::BothGenders(form) => write!(
                f,
                "{form} writes both genders at once; use a condition on $gn4 with the feminine form first, or a phrasing that shows no gender"
            ),
            Self::TermNotUsed { term, translation } => write!(
                f,
                "the terms translate {term:?} as {translation:?}, which does not seem to be used"
            ),
            Self::GenderNotVaried => f.write_str(
                "the source varies with the player character's gender and the translation does not; make sure nothing in it agrees with the player character's gender",
            ),
            Self::GenderInOtherLanguages => f.write_str(
                "the French or German line varies with the player character's gender; check whether a word about the player character needs a condition on $gn4",
            ),
            Self::MachinePhrasing(phrases) => {
                write!(f, "reads machine-written: {}", phrases.join(", "))
            }
            Self::StaleTermException(term) => write!(
                f,
                "the string has an exception for the term {term:?}, which its source does not have; remove the exception"
            ),
        }
    }
}

/// Languages written in Cyrillic, by their primary language subtag.
const CYRILLIC_LANGUAGES: [&str; 10] = ["ru", "uk", "be", "bg", "sr", "mk", "kk", "ky", "tg", "mn"];

fn is_cyrillic(letter: char) -> bool {
    matches!(letter, '\u{0400}'..='\u{052F}')
}

fn is_latin(letter: char) -> bool {
    letter.is_ascii_alphabetic()
        || matches!(letter, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

fn is_greek(letter: char) -> bool {
    matches!(letter, '\u{0370}'..='\u{03FF}')
}

/// The text of a string with each tag as a space, where only words are read.
fn words_of(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut depth = 0_usize;
    let mut escaped = false;
    for character in text.chars() {
        match character {
            '\\' if !escaped => escaped = true,
            '<' if !escaped => {
                depth += 1;
                plain.push(' ');
            }
            '>' if !escaped && depth > 0 => depth -= 1,
            _ => {
                escaped = false;
                if depth == 0 {
                    plain.push(character);
                }
            }
        }
    }
    plain
}

/// Slips of letters in a translation into a language written in Cyrillic:
/// stress and other combining marks, a word that mixes Cyrillic with Latin
/// or Greek letters, and letters of another writing system. What the source
/// has itself, such as the æ of Pandæmonium or a line of Japanese, is not a
/// slip.
fn letter_slips(target_language: &str, source: &str, text: &str) -> Vec<Issue> {
    let primary = target_language.split('-').next().unwrap_or_default();
    if !CYRILLIC_LANGUAGES
        .iter()
        .any(|language| language.eq_ignore_ascii_case(primary))
    {
        return Vec::new();
    }
    let plain = words_of(text);
    let mut slips = Vec::new();
    if let Some(mark) = plain
        .chars()
        .find(|c| matches!(c, '\u{0300}'..='\u{036F}') && !source.contains(*c))
    {
        slips.push(Issue::Mark(mark));
    }
    for word in plain.split(|c: char| !c.is_alphanumeric() && !matches!(c, '\u{0300}'..='\u{036F}'))
    {
        // A plain Latin letter never belongs in a Cyrillic word; a letter
        // such as æ does when the source spells the name with it.
        let mixed = word.chars().any(is_cyrillic)
            && word.chars().any(|c| {
                c.is_ascii_alphabetic() || ((is_latin(c) || is_greek(c)) && !source.contains(c))
            });
        let other = word.chars().any(|c| {
            c.is_alphabetic()
                && !is_cyrillic(c)
                && !is_latin(c)
                && !is_greek(c)
                && !source.contains(c)
        });
        if (mixed || other) && !source.contains(word) {
            slips.push(Issue::MixedAlphabets(word.to_owned()));
            break;
        }
    }
    slips
}

/// Russian forms that write both genders at once, such as `готов(а)`.
const BOTH_GENDERS: [&str; 6] = ["(а)", "(ла)", "(ая)", "(на)", "(ен)", "(ой)"];

/// Checks one translation against its source and the project knowledge.
/// `extracted` are the entry's `#.` lines, with the other client languages;
/// `exceptions` the terms a person decided do not apply to the string.
#[must_use]
pub fn check_translation(
    knowledge: &Knowledge,
    target_language: &str,
    source: &str,
    text: &str,
    extracted: &[String],
    exceptions: &[String],
) -> Verdict {
    let mut issues = Vec::new();
    if text.contains('\n') && !source.contains('\n') {
        issues.push(Issue::LineBreak);
    }
    if let Err(errors) = aeria_se::check_assisted_structure(source, text) {
        issues.extend(
            errors
                .into_iter()
                .map(|error| Issue::Structure(error.message)),
        );
    }
    let terms = knowledge.terms.review(source, text, exceptions);
    issues.extend(
        terms
            .forbidden
            .iter()
            .map(|(entry, variant)| Issue::ForbiddenTerm {
                term: entry.term.clone(),
                translation: entry.translation.clone(),
                variant: (*variant).to_owned(),
            }),
    );
    issues.extend(letter_slips(target_language, source, text));
    let russian = target_language.eq_ignore_ascii_case("ru");
    if russian
        && let Some(form) = BOTH_GENDERS
            .iter()
            .find(|form| text.contains(*form) && !source.contains(*form))
    {
        issues.push(Issue::BothGenders((*form).to_owned()));
    }
    issues.extend(terms.unused.iter().map(|entry| Issue::TermNotUsed {
        term: entry.term.clone(),
        translation: entry.translation.clone(),
    }));
    issues.extend(
        terms
            .stale
            .iter()
            .map(|term| Issue::StaleTermException((*term).to_owned())),
    );
    if source.contains("$gn4") && !text.contains("$gn4") {
        issues.push(Issue::GenderNotVaried);
    } else if russian
        && !source.contains("$gn4")
        && !text.contains("$gn4")
        && extracted.iter().any(|line| {
            (line.starts_with("fr: ") || line.starts_with("de: ")) && line.contains("$gn4")
        })
    {
        issues.push(Issue::GenderInOtherLanguages);
    }
    let phrasing = machine_phrasing(target_language, text);
    if !phrasing.is_empty() {
        issues.push(Issue::MachinePhrasing(
            phrasing.into_iter().map(ToString::to_string).collect(),
        ));
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
        let verdict = check_translation(
            knowledge,
            target_language,
            &entry.source,
            &entry.translation,
            &entry.extracted,
            &entry.term_exceptions,
        );
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

    #[test]
    fn slips_of_letters_are_problems_unless_the_source_has_them() {
        let knowledge = Knowledge::default();
        let problems = |source: &str, text: &str| {
            check_translation(&knowledge, "ru", source, text, &[], &[]).problems
        };
        for (source, text) in [
            ("Elezen boy", "юный эле\u{301}зен"),
            ("deep palace sarcosuchus", "сарcosух Дворца мёртвых"),
            ("Thancred", "Тан\u{915}\u{94D}ред"),
            ("Pharmakon", "Фарма\u{3BA}он"),
            ("Mauto", "Маут\u{14D}"),
        ] {
            assert!(!problems(source, text).is_empty(), "{text}");
        }
        for (source, text) in [
            ("Pand\u{e6}monium: Abyssos", "Панд\u{e6}мониум: Абиссос"),
            ("Seed of Magic Alpha", "Зёрна магии \u{3B1}"),
            (
                "Use <ui-color 500>Fast Blade</ui-color>.",
                "Используйте <ui-color 500>Быстрый клинок</ui-color>.",
            ),
            ("Hingan andon lamp", "Хинганский фонарь «andon»"),
            ("A 4K display", "Монитор 4K"),
            (
                "ALIASES:<br>/qchat<br>USAGE:",
                "ПСЕВДОНИМЫ:<br>/qchat<br>ИСПОЛЬЗОВАНИЕ:",
            ),
        ] {
            assert!(
                problems(source, text).is_empty(),
                "{text}: {:?}",
                problems(source, text)
            );
        }
        assert!(
            check_translation(&knowledge, "de", "Hello", "Hall\u{f6}", &[], &[])
                .problems
                .is_empty()
        );
    }

    #[test]
    fn broken_macros_and_both_genders_are_problems() {
        let knowledge = Knowledge::default();
        assert!(
            check_translation(&knowledge, "ru", "Hello.", "Привет.", &[], &[])
                .problems
                .is_empty()
        );
        let verdict =
            check_translation(&knowledge, "ru", "You are ready.", "Ты готов(а).", &[], &[]);
        assert_eq!(verdict.problems.len(), 1, "{verdict:?}");
        let verdict = check_translation(&knowledge, "ru", "Hello.", "При\nвет.", &[], &[]);
        assert_eq!(verdict.problems.len(), 1, "{verdict:?}");
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
