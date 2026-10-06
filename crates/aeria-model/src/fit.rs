//! A check of an answer that only a run can make: that a translation
//! belongs to its string.
//!
//! A request numbers its strings, and in a long run of short, similar strings
//! the model can slip and answer one string with the next one's translation.
//! Each answer therefore repeats the first words of its source; an answer
//! whose words are not the start of its own source is refused.

use aeria_po::length::scan;

/// Whether `start`, the words an answer repeats, are words of the string's
/// source, comparing letters and digits only, case ignored. The words may
/// come from inside its macros or after a conditional part the model left
/// out, so they need not begin the source, and they need not follow each
/// other in it: the model leaves out the macros and branches between them
/// (`you see` of `<if ($gs1 == $gs2)>you<else>…</if> <if …>see<else>sees</if>`).
/// They must occur in their order, so another string's words do not match.
/// An empty `start` is accepted only for a source that begins with a macro,
/// whose first words the model cannot tell.
#[must_use]
pub fn matches_start(start: &str, source: &str) -> bool {
    let start_text = start;
    let all = letters(source, true);
    if all.is_empty() {
        return true;
    }
    let start = letters(start, true);
    if start.is_empty() {
        return source
            .trim_start_matches(|c: char| c.is_whitespace())
            .starts_with('<');
    }
    if all.contains(&start) {
        return true;
    }
    let mut rest = all.as_str();
    start_words(start_text).all(|word| match rest.find(&word) {
        Some(at) => {
            rest = &rest[at + word.len()..];
            true
        }
        None => false,
    })
}

/// The words of `text`, lowercase, letters and digits only.
fn start_words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// The lowercase letters and digits of macro text, with or without those
/// inside macro tags.
fn letters(text: &str, with_macros: bool) -> String {
    let mut out = String::new();
    scan(text, |c, in_tag| {
        if (with_macros || !in_tag) && c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_must_repeat_the_start_of_its_own_source() {
        assert!(matches_start("Loot", "Loot"));
        assert!(matches_start("Display Rules", "<icon2 11> Display Rules"));
        assert!(matches_start(
            "you tell thancred",
            "You tell Thancred and Alisaie of your reunion."
        ));
        assert!(matches_start("Type 1:", "Type 1: Ignore Depth"));
        let waves = "<capitalize><if ($gs1 == $gs2)>you<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if></if></capitalize> <if ($gs1 == $gs2)>wave<else>waves</if> to <if ($gs1 == $gs3)>you<else>{$gs3}</if>.";
        assert!(
            matches_start("you wave to", waves),
            "words with macros between them"
        );
        assert!(matches_start("You waves", waves));
        assert!(
            !matches_start("you see", waves),
            "another emote's words do not match"
        );
        assert!(
            !matches_start("to wave", waves),
            "the words keep their order"
        );
        assert!(!matches_start("Play Style", "Loot"));
        assert!(!matches_start("Objective", "None"));
        assert!(!matches_start("", "Loot"));
        assert!(matches_start("", "<icon 77>"));
        assert!(matches_start("!", "!"));
        assert!(matches_start(
            "Crystalline Conflict",
            "<if ($n1 > 0)>Crystalline Conflict<else>???</if>"
        ));
        assert!(matches_start(
            "HP:",
            "<sheet ActStr $n1 0><br><if $n2>Lv. <num $n2> </if>HP: <num $n3>"
        ));
        assert!(matches_start(
            "Average-sized",
            "\u{3000}<if ($n2 > 0)>Average-sized<else>Large-sized</if> = <num $n1> pts"
        ));
        assert!(matches_start(
            "<capitalize>",
            "<capitalize><sheet ActStr $n1 0></capitalize><br>MP: <num $n3>"
        ));
        assert!(matches_start(
            "",
            r#"<sheet Addon 15955 0 "<noun-en PlaceName 2 $n18 2 1>" $n14 $n12>"#
        ));
    }
}
