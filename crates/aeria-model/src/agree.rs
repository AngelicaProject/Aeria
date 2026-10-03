//! Words that agree with a person of a message. A log message tells "you"
//! from someone else by comparing the player's name with another's
//! (`<if ($gs1 == $gs2)>`), and English chooses a word the same way
//! (`<if ($gs1 == $gs2)>scan<else>scans</if>`). A translation that keeps
//! only the name in such a comparison and one form of the word after it
//! agrees with only one of the two, as «Вы … осматривают».

use std::collections::BTreeMap;

use aeria_se::{ExprKind, ExprSyntax, SyntaxKind, SyntaxNode, parse};

/// The type byte of a global text parameter (`$gs…`).
const GLOBAL_TEXT: u8 = 0xEB;
/// The comparison `==`.
const EQUAL: u8 = 0xE4;

/// A comparison of the player's name with another person's: the person's
/// index (2 for `$gs2`) and the two branches.
struct Comparison<'a> {
    person: u32,
    then: &'a ExprSyntax,
    otherwise: &'a ExprSyntax,
}

fn global_text(expr: &ExprSyntax) -> Option<u32> {
    match &expr.kind {
        ExprKind::Param(GLOBAL_TEXT, index) => match index.kind {
            ExprKind::Int(index) => Some(index),
            _ => None,
        },
        _ => None,
    }
}

/// Every comparison of `$gs1` with another `$gs…` in `nodes`, nested ones
/// included.
fn comparisons<'a>(nodes: &'a [SyntaxNode], found: &mut Vec<Comparison<'a>>) {
    for node in nodes {
        let SyntaxKind::Macro(syntax) = &node.kind else {
            continue;
        };
        if syntax.spec.is_some_and(|spec| spec.name == "if")
            && let [condition, then, otherwise, ..] = syntax.args.as_slice()
            && let ExprKind::Compare(EQUAL, left, right) = &condition.kind
        {
            let person = match (global_text(left), global_text(right)) {
                (Some(1), Some(other)) | (Some(other), Some(1)) if other != 1 => Some(other),
                _ => None,
            };
            if let Some(person) = person {
                found.push(Comparison {
                    person,
                    then,
                    otherwise,
                });
            }
        }
        for arg in &syntax.args {
            if let ExprKind::Str(inner) = &arg.kind {
                comparisons(inner, found);
            }
        }
    }
}

/// The words of a branch that is only text, such as `scans`.
fn plain_words(branch: &ExprSyntax) -> Option<String> {
    let ExprKind::Str(nodes) = &branch.kind else {
        return None;
    };
    let mut words = String::new();
    for node in nodes {
        match &node.kind {
            SyntaxKind::Text(text) => words.push_str(text),
            _ => return None,
        }
    }
    let words = words.trim().to_owned();
    words.chars().any(char::is_alphabetic).then_some(words)
}

/// Whether a branch has words of its own, in its text or in the translatable
/// text of the macros in it; a macro's sheet name or other arguments are
/// not words of the branch.
fn has_words(branch: &ExprSyntax) -> bool {
    let ExprKind::Str(nodes) = &branch.kind else {
        return false;
    };
    nodes.iter().any(|node| match &node.kind {
        SyntaxKind::Text(text) => text.chars().any(char::is_alphabetic),
        SyntaxKind::Macro(syntax) => syntax.args.iter().enumerate().any(|(index, arg)| {
            syntax
                .spec
                .is_some_and(|spec| spec.is_translatable_arg(index))
                && has_words(arg)
        }),
        _ => false,
    })
}

/// The words the source chooses by whether a person is the player, by
/// person: `{2: ("scan", "scans")}`.
fn chosen_words(source: &str) -> BTreeMap<u32, (String, String)> {
    let document = parse(source);
    let mut found = Vec::new();
    comparisons(document.nodes(), &mut found);
    let mut chosen = BTreeMap::new();
    for comparison in found {
        if let (Some(then), Some(otherwise)) = (
            plain_words(comparison.then),
            plain_words(comparison.otherwise),
        ) && then != otherwise
        {
            chosen.entry(comparison.person).or_insert((then, otherwise));
        }
    }
    chosen
}

/// The problems of a translation that chooses no word by a person the
/// source chooses one by: the translation needs a comparison with that
/// person whose branches both have words, so that a word agreeing with the
/// person is chosen with it. Repeating a whole phrase in each branch always
/// satisfies it.
#[must_use]
pub fn agreement_problems(source: &str, translation: &str) -> Vec<String> {
    let chosen = chosen_words(source);
    if chosen.is_empty() {
        return Vec::new();
    }
    let document = parse(translation);
    let mut found = Vec::new();
    comparisons(document.nodes(), &mut found);
    chosen
        .into_iter()
        .filter(|(person, _)| {
            !found.iter().any(|comparison| {
                comparison.person == *person
                    && has_words(comparison.then)
                    && has_words(comparison.otherwise)
            })
        })
        .map(|(person, (then, otherwise))| {
            format!(
                "the source chooses \"{then}\" or \"{otherwise}\" by whether $gs{person} is the \
                 player character, and the translation chooses no word that way: a word that \
                 agrees with that person, such as the verb after them, agrees differently with \
                 \"you\" and with someone else, so choose it in the branches of <if ($gs1 == \
                 $gs{person})>, or repeat the whole phrase in each branch"
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCAN: &str = "<capitalize><if ($gs1 == $gs2)>you<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if></if></capitalize> <if ($gs1 == $gs2)>scan<else>scans</if> the area around <if ($gs1 == $gs3)>you<else>{$gs3}</if>.";

    #[test]
    fn one_form_after_the_person_is_a_problem() {
        let problems = agreement_problems(
            SCAN,
            "<capitalize><if ($gs1 == $gs2)>Вы<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if></if></capitalize> осматривают местность вокруг <if ($gs1 == $gs3)>вас<else>{$gs3}</if>.",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("\"scan\" or \"scans\""),
            "{}",
            problems[0]
        );
        assert!(problems[0].contains("$gs2"), "{}", problems[0]);
    }

    #[test]
    fn a_word_chosen_in_the_branches_passes() {
        for translation in [
            // the verb in a condition of its own
            "<capitalize><if ($gs1 == $gs2)>Вы<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if></if></capitalize> <if ($gs1 == $gs2)>осматриваете<else>осматривает</if> местность вокруг <if ($gs1 == $gs3)>вас<else>{$gs3}</if>.",
            // the person and the verb in one condition
            "<capitalize><if ($gs1 == $gs2)>Вы осматриваете<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if> осматривает</if></capitalize> местность вокруг <if ($gs1 == $gs3)>вас<else>{$gs3}</if>.",
            // the whole phrase repeated
            "<if ($gs1 == $gs2)>Местность осмотрена вами<else>Местность осмотрена: <if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if></if>.",
        ] {
            assert!(
                agreement_problems(SCAN, translation).is_empty(),
                "{translation}"
            );
        }
    }

    #[test]
    fn a_source_without_a_word_choice_asks_for_nothing() {
        let source = "<if ($gs1 == $gs2)>you<else>{$gs2}</if> left.";
        assert!(chosen_words(source).is_empty());
        assert!(agreement_problems(source, "Ушли.").is_empty());
        assert!(agreement_problems("You left.", "Вы ушли.").is_empty());
    }
}
