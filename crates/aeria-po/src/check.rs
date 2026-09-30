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
}

/// Russian forms that write both genders at once, such as `готов(а)`.
const BOTH_GENDERS: [&str; 6] = ["(а)", "(ла)", "(ая)", "(на)", "(ен)", "(ой)"];

/// Checks one translation against its source and the project knowledge.
/// `extracted` are the entry's `#.` lines, with the other client languages.
#[must_use]
pub fn check_translation(
    knowledge: &Knowledge,
    target_language: &str,
    source: &str,
    text: &str,
    extracted: &[String],
) -> Verdict {
    let mut verdict = Verdict::default();
    if text.contains('\n') && !source.contains('\n') {
        verdict.problems.push(
            "the translation has a line break the source does not; the game breaks lines with <br>"
                .to_owned(),
        );
    }
    if let Err(errors) = aeria_se::check_assisted_structure(source, text) {
        verdict
            .problems
            .extend(errors.into_iter().map(|error| error.message));
    }
    verdict
        .problems
        .extend(knowledge.terms.forbidden_in(source, text));
    let russian = target_language.eq_ignore_ascii_case("ru");
    if russian
        && let Some(form) = BOTH_GENDERS
            .iter()
            .find(|form| text.contains(*form) && !source.contains(*form))
    {
        verdict.problems.push(format!(
            "{form} writes both genders at once; use a condition on $gn4 with the feminine form first, or a phrasing that shows no gender"
        ));
    }
    verdict
        .advice
        .extend(knowledge.terms.missing_in(source, text));
    if source.contains("$gn4") && !text.contains("$gn4") {
        verdict.advice.push("the source varies with the player character's gender and the translation does not; make sure nothing in it agrees with the player character's gender".to_owned());
    } else if russian
        && !source.contains("$gn4")
        && !text.contains("$gn4")
        && extracted.iter().any(|line| {
            (line.starts_with("fr: ") || line.starts_with("de: ")) && line.contains("$gn4")
        })
    {
        verdict.advice.push("the French or German line varies with the player character's gender; check whether a word about the player character needs a condition on $gn4".to_owned());
    }
    let phrasing = machine_phrasing(target_language, text);
    if !phrasing.is_empty() {
        verdict
            .advice
            .push(format!("reads machine-written: {}", phrasing.join(", ")));
    }
    verdict
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
    fn broken_macros_and_both_genders_are_problems() {
        let knowledge = Knowledge::default();
        assert!(
            check_translation(&knowledge, "ru", "Hello.", "Привет.", &[])
                .problems
                .is_empty()
        );
        let verdict = check_translation(&knowledge, "ru", "You are ready.", "Ты готов(а).", &[]);
        assert_eq!(verdict.problems.len(), 1, "{verdict:?}");
        let verdict = check_translation(&knowledge, "ru", "Hello.", "При\nвет.", &[]);
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
