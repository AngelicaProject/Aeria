//! The checks of a translation and of a file. Aeria runs them when a
//! translation is saved or machine-translated, and over the files of a
//! project; a translation with a problem is never saved.

use std::collections::BTreeSet;

use aeria_knowledge::Knowledge;
use aeria_knowledge::rules::machine_phrasing;

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

/// One finding of the checks of a translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Issue {
    /// A line break the source does not have.
    LineBreak,
    /// The structure policy refused the macros (`aeria_se`); the message is
    /// written for the model that produced the translation.
    Structure(String),
    /// A stress or other combining mark the source does not have.
    Mark(char),
    /// A word that mixes alphabets, or has letters of another writing system.
    MixedAlphabets(String),
    /// A Russian form that writes both genders at once, such as `(а)`.
    BothGenders(String),
    /// Advice: a word written twice in a row, in a reading of the
    /// translation's conditions (see [`aeria_se::MacroString::readings`]).
    RepeatedWord(String),
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
    /// Advice: an interface label shows more characters than the longest
    /// official localization of it (see [`crate::length`]).
    LabelTooLong { length: usize, max: usize },
    /// Advice: the name of a world object has more bytes than the game
    /// shows; the game cuts it (see [`crate::length`]).
    NameTooLong { length: usize, max: usize },
}

impl Issue {
    /// What groups issues for a summary: the kind, with the term for term
    /// issues (`termNotUsed:pugilist`).
    #[must_use]
    pub fn group(&self) -> String {
        match self {
            Self::LineBreak => "lineBreak".to_owned(),
            Self::Structure(_) => "structure".to_owned(),
            Self::Mark(_) => "mark".to_owned(),
            Self::MixedAlphabets(_) => "mixedAlphabets".to_owned(),
            Self::BothGenders(_) => "bothGenders".to_owned(),
            Self::RepeatedWord(_) => "repeatedWord".to_owned(),
            Self::TermNotUsed { term, .. } => format!("termNotUsed:{}", term.to_lowercase()),
            Self::GenderNotVaried => "genderNotVaried".to_owned(),
            Self::GenderInOtherLanguages => "genderInOtherLanguages".to_owned(),
            Self::MachinePhrasing(_) => "machinePhrasing".to_owned(),
            Self::StaleTermException(_) => "staleTermException".to_owned(),
            Self::LabelTooLong { .. } => "labelTooLong".to_owned(),
            Self::NameTooLong { .. } => "nameTooLong".to_owned(),
        }
    }

    /// A problem stops a translation from being saved and exported; the rest
    /// is advice.
    #[must_use]
    pub fn is_problem(&self) -> bool {
        !matches!(
            self,
            Self::TermNotUsed { .. }
                | Self::RepeatedWord(_)
                | Self::GenderNotVaried
                | Self::GenderInOtherLanguages
                | Self::MachinePhrasing(_)
                | Self::StaleTermException(_)
                | Self::LabelTooLong { .. }
                | Self::NameTooLong { .. }
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
            Self::RepeatedWord(word) => write!(
                f,
                "«{word}» is written twice in a row; remove one, or check the branches of a condition"
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
            Self::LabelTooLong { length, max } => write!(
                f,
                "the translation shows {length} characters and the interface fits {max}"
            ),
            Self::NameTooLong { length, max } => write!(
                f,
                "the name is {length} bytes and the game shows at most {max} over the character or object"
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

/// A word the translation writes twice in a row, with only spaces between,
/// in either reading of its conditions; `None` when there is none or the
/// source repeats a word itself, as a stutter or a call does. A word
/// repeated with a capital letter both times is a name, such as a
/// Lalafell's (Гун Гун), and a line break or a value filled in at runtime
/// parts the words around it.
fn repeated_word(source: &str, text: &str) -> Option<String> {
    let repeated = |text: &str| {
        aeria_se::parse(text)
            .readings()
            .into_iter()
            .find_map(|reading| {
                let mut previous: Option<(String, bool)> = None;
                let mut gap_is_space = false;
                let mut word = String::new();
                let mut found = None;
                for character in reading.chars().chain(std::iter::once('.')) {
                    if character.is_alphabetic() || character == '-' || character == '\'' {
                        word.push(character);
                        continue;
                    }
                    if !word.is_empty() && word.chars().any(char::is_alphabetic) {
                        let capital = word.chars().next().is_some_and(char::is_uppercase);
                        let lower = word.to_lowercase();
                        if gap_is_space
                            && previous.as_ref().is_some_and(|(seen, was_capital)| {
                                *seen == lower && !(capital && *was_capital)
                            })
                        {
                            found = Some(word.clone());
                            break;
                        }
                        previous = Some((lower, capital));
                        gap_is_space = true;
                    }
                    word.clear();
                    if character == '\n' {
                        gap_is_space = false;
                        previous = None;
                    } else if !character.is_whitespace() {
                        gap_is_space = false;
                        if !character.is_alphanumeric() {
                            previous = None;
                        }
                    }
                }
                found
            })
    };
    repeated(source).is_none().then(|| repeated(text)).flatten()
}

/// Russian forms that write both genders at once, such as `готов(а)`.
const BOTH_GENDERS: [&str; 6] = ["(а)", "(ла)", "(ая)", "(на)", "(ен)", "(ой)"];

/// Checks `text` as the translation of `entry` against its source, its `#.`
/// lines (with the other client languages), the terms a person decided do
/// not apply to it, its length budget, and the project knowledge.
#[must_use]
pub fn check_translation(
    knowledge: &Knowledge,
    target_language: &str,
    entry: &Entry,
    text: &str,
) -> Verdict {
    let source = entry.source.as_str();
    let extracted = entry.extracted.as_slice();
    let exceptions = entry.term_exceptions.as_slice();
    let mut issues = Vec::new();
    if text.contains('\n') && !source.contains('\n') {
        issues.push(Issue::LineBreak);
    }
    // The same string in the other client languages: game data they use
    // in place of the source's may stand in the translation.
    let localizations: Vec<&str> = extracted
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
    let terms = knowledge.terms.review(source, text, exceptions);
    issues.extend(letter_slips(target_language, source, text));
    let russian = target_language.eq_ignore_ascii_case("ru");
    if russian
        && let Some(form) = BOTH_GENDERS
            .iter()
            .find(|form| text.contains(*form) && !source.contains(*form))
    {
        issues.push(Issue::BothGenders((*form).to_owned()));
    }
    issues.extend(repeated_word(source, text).map(Issue::RepeatedWord));
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
    if let Some(budget) = length_budget(entry) {
        let length = budget.length_of(text);
        let max = budget.max;
        if length > max {
            issues.push(match budget.unit {
                Unit::Characters => Issue::LabelTooLong { length, max },
                Unit::Bytes => Issue::NameTooLong { length, max },
            });
        }
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
    fn slips_of_letters_are_problems_unless_the_source_has_them() {
        let knowledge = Knowledge::default();
        let problems =
            |source: &str, text: &str| check(&knowledge, "ru", source, text, &[]).problems;
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
            check(&knowledge, "de", "Hello", "Hall\u{f6}", &[])
                .problems
                .is_empty()
        );
    }

    #[test]
    fn broken_macros_and_both_genders_are_problems() {
        let knowledge = Knowledge::default();
        assert!(
            check(&knowledge, "ru", "Hello.", "Привет.", &[])
                .problems
                .is_empty()
        );
        let verdict = check(&knowledge, "ru", "You are ready.", "Ты готов(а).", &[]);
        assert_eq!(verdict.problems.len(), 1, "{verdict:?}");
        let verdict = check(&knowledge, "ru", "Hello.", "При\nвет.", &[]);
        assert_eq!(verdict.problems.len(), 1, "{verdict:?}");
    }

    #[test]
    fn a_word_written_twice_in_a_row_is_advice() {
        let knowledge = Knowledge::default();
        let repeated = |source: &str, text: &str| {
            check(&knowledge, "ru", source, text, &[])
                .issues
                .into_iter()
                .find_map(|issue| match issue {
                    Issue::RepeatedWord(word) => Some(word),
                    _ => None,
                })
        };
        let source = "Or should I call you the hero of the Scions?";
        assert_eq!(
            repeated(
                source,
                "Или назвать тебя героем <if $gn4>героиней<else>героем</if> Потомков?"
            )
            .as_deref(),
            Some("героем")
        );
        assert_eq!(repeated(source, "Я в в городе.").as_deref(), Some("в"));
        for text in [
            "Да, да, конечно.",
            "Ну-ну, посмотрим.",
            "Ты <if $gn4>сказала<else>сказал</if>, сказал<if $gn4>а</if> же.",
            "Нет... нет!",
            "Мой сын Гун Гун очень меня радует.",
            "3. Активное<br>Активное овоо можно захватить.",
        ] {
            assert_eq!(repeated(source, text), None, "{text}");
        }
        assert_eq!(repeated("No no no!", "Нет нет нет!"), None);
    }

    #[test]
    fn a_translation_longer_than_its_budget_is_advice() {
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
        let loot = entry("Addon:1:0:0", "Loot", &["de: Beutegut", "fr: Butin"]);
        assert_eq!(
            check_translation(&knowledge, "ru", &loot, "Военные трофеи").issues,
            vec![Issue::LabelTooLong { length: 14, max: 8 }]
        );
        assert_eq!(
            Issue::LabelTooLong { length: 14, max: 8 }.group(),
            "labelTooLong"
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
