//! Sounds and stage directions in angle brackets. In macro text `\<` is a
//! literal `<`, so `\<sigh>` and `\<click>` are words of the English
//! localization, not macros: a translation renders them in its language, as
//! a sound of the line (Эх…) or in brackets, as the project's style says. A
//! model that takes them for macros keeps the English word.

/// Placeholders of chat and help texts, which the game fills in and which
/// stay as they are: `<t>` is the target, `<me>` the player.
const PLACEHOLDERS: &[&str] = &[
    "t", "tt", "me", "mo", "f", "r", "pos", "hp", "hpp", "mp", "mpp", "tp", "tpp", "target",
    "attack", "bind", "stop", "square", "circle", "cross", "triangle", "se", "wait", "lockon",
    "class", "job", "flag", "mark",
];

/// Whether a bracketed word is a sound or stage direction of the English
/// localization (`sigh`, `click`, `buzzzzzz`): lowercase letters, three or
/// more, and not a placeholder. A word with digits or capitals is a
/// placeholder (`<attack1>`) or a name (`<Cloud Strife>`).
fn is_sound(word: &str) -> bool {
    word.chars().filter(char::is_ascii_lowercase).count() >= 3
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || matches!(c, '\'' | '-' | ' '))
        && !PLACEHOLDERS.contains(&word)
}

/// The words in angle brackets of macro text: `\<sigh>` gives `sigh`.
fn bracketed(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\\<") {
        rest = &rest[at + 2..];
        let Some(end) = rest.find('>') else { break };
        let word = &rest[..end];
        if !word.is_empty() && word.chars().count() <= 30 && !word.contains(['<', '\\']) {
            words.push(word);
        }
        rest = &rest[end + 1..];
    }
    words
}

/// The problems of a translation that keeps a sound of the source as it
/// is: the same word in angle brackets, case ignored.
#[must_use]
pub fn sound_problems(source: &str, translation: &str) -> Vec<String> {
    let kept: Vec<&str> = bracketed(translation);
    let mut problems: Vec<String> = bracketed(source)
        .into_iter()
        .filter(|word| is_sound(word))
        .filter(|word| kept.iter().any(|mine| mine.eq_ignore_ascii_case(word)))
        .map(|word| {
            format!(
                "\\<{word}> is a sound or stage direction of the English localization, \
                 not a macro: render it in the target language as the instructions say, as a \
                 sound word of the line (\\<sigh> Эх…), or leave it out, but never keep the \
                 English word"
            )
        })
        .collect();
    problems.dedup();
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sound_left_in_english_is_a_problem() {
        let source = r"Gods, I think I'm<br>in love... \<sigh> \<sigh>";
        let problems = sound_problems(source, r"Боги, кажется, я<br>влюбилась... \<sigh> \<sigh>");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with(r"\<sigh>"), "{}", problems[0]);
        assert!(sound_problems(source, r"Боги, кажется, я<br>влюбилась... \<вздох>").is_empty());
        assert!(
            sound_problems(source, "Боги, кажется, я<br>влюбилась...").is_empty(),
            "dropping it is the translation's choice"
        );
        assert!(sound_problems("No sound here.", r"Без звука \<sigh>.").is_empty());
    }

    #[test]
    fn placeholders_and_names_stay_as_they_are() {
        for source in [
            r"/tell \<t>",
            r"Use \<attack1>.",
            r"\<hp> \<mp>",
            r"\<target>",
            r"\<Cloud Strife>",
        ] {
            assert!(sound_problems(source, source).is_empty(), "{source}");
        }
    }

    #[test]
    fn brackets_are_read_as_text_not_macros() {
        assert_eq!(
            bracketed(r"\<click> \<click> <br>(Seen it?)"),
            ["click", "click"]
        );
        assert_eq!(bracketed(r"<if $gn4>a<else>b</if> \<wink>"), ["wink"]);
        assert!(bracketed(r"\<").is_empty());
    }
}
