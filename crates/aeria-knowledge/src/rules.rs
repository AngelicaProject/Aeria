//! The rules every translation follows, whoever writes it: what the other
//! client languages are for, how a line reads as if written in the target
//! language, how the player character is addressed, and the signs of
//! machine-written text.

/// What the game's texts in its client languages are, and what each is
/// evidence for.
pub const ORIGINAL_TEXT: &str = "\
FINAL FANTASY XIV is written in Japanese. The English, German, and French texts are \
three finished localizations, and each made its own creative choices: names (the \
tavern 溺れた海豚亭, \"the drowned dolphin\", is the Drowning Wench in English, the \
Dauphin noyé in French, and Zur Ertränkten Sorge, \"drowned sorrow\", in German), jokes, \
and how characters sound (accent, dialect, class, pomp, tics). This project is another \
localization of the game: the Japanese shows what the writers meant and how a character \
speaks (first-person pronoun, sentence endings, politeness); the localizations show how \
each made the line work for its players, including decisions one language hides, such \
as tu/vous and du/Sie between speakers and toward the player character, genders, and \
register. Make the project's own choices from all of them; copy none, and never make a \
character flatter than the original and the localizations make them.
Content comes only from the source line, the project's working text. Never move a word, \
detail, sentence, or example from another language into a line, even when the Japanese \
says more or something else; the other languages decide only tone, voice, address, \
gender, and how to read an ambiguous source line. When the Japanese and the source \
differ in a way that matters, follow the project's style or ask.";

/// How every translation should read.
pub const TRANSLATION_STYLE: &str = "\
Translation style. These are defaults: the project knowledge takes precedence wherever \
it says otherwise.
- A translation reads as if it had been written in the target language for this game, \
and a player should not notice that it is a translation. Carry over meaning, tone, and \
intent rather than words, in the target language's own word order and syntax. Split or \
merge sentences when that reads better, as long as nothing is added or lost.
- Watch for translationese: calqued idioms and phrasal verbs, chains of nouns where the \
target language uses a verb, passives it would make active, pronouns and possessives it \
would leave out, and the source's quotation marks, dashes, ellipses, and title \
capitalization in place of the target language's conventions.
- Keep the register and emotion of the line. An idiom, joke, or pun becomes a natural \
equivalent that does the same job; a curse stays a curse, a decree stays formal, a child \
sounds like a child, and speech sounds spoken. Interface labels and names stay as short \
as similar strings in the project, and system messages keep the game's usual phrasing.
- Do not add explanations, softening, or flourish the line does not have, and do not \
flatten a line that is dramatic or poetic.
- Stock AI words, \"not X, but Y\" contrasts, and lists of three stand out in a game; use \
them only where the line itself has them.
- Machine-written text reads dry for known reasons; avoid each: written register in \
speech (complete, tidy sentences joined by \"therefore\" and \"however\" where people cut \
phrases, leave things unsaid, and put the important word first), officialese and verbal \
nouns where a verb would do, the safest and most frequent word where this speaker would \
use a concrete one of their own, the particles and interjections of spoken language left \
out, the same rhythm and the same \"X: explanation\" or \"X — Y\" shape line after line, \
irony, rudeness, pomp, or clumsiness smoothed into neutral politeness, and every source \
phrase carried over one to one in the same order.
- Before writing, reread each translation on its own as the player would see it, \
aloud: is this how this person talks? If it sounds translated or machine-written, \
rewrite it.";

/// How translations refer to the player character, whose gender the game
/// knows only at runtime.
pub const PLAYER_CHARACTER: &str = "\
The player character. Players choose their character's gender, so a translation never \
assumes one.
- In quests and cutscenes, \"you\" is the player character unless the scene shows the \
speaker is talking to someone else. Journal entries and objectives always speak to the \
player character. Lines about the adventurer, the Warrior of Light, or the player's name \
are about the player character too.
- In a language with grammatical gender, every word that agrees with the player \
character's gender, such as a past-tense verb, an adjective, a participle, a noun for a \
person, or a pronoun, goes in a condition on $gn4 with the feminine form first. Wrap \
whole words or phrases, not endings. In Russian: «Ты <if $gn4>готова<else>готов</if>?», \
«<if $gn4>Ты сама всё видела<else>Ты сам всё видел</if>», «простая \
<if $gn4>посыльная<else>посыльный</if>». A phrasing where nothing agrees with the gender \
is equally good when it reads naturally.
- Never write both forms with a slash or parentheses, such as готов(а) or готов/готова, \
and never choose one gender for the player character.
- When the source has a condition on $gn4, the translation has one as well or is \
phrased so that nothing depends on the gender. Never keep only one of its branches.
- When the French or German line varies with the player character's gender and the \
source does not, the target language most likely needs a condition there too.
- Speakers keep their own gender: a condition on $gn4 is only for words about the \
player character.
- A comparison with the player's name, such as <if ($gs1 == $gs2)>, tells whether a \
person a message is about is the player character: its first branch is about the \
player character, usually as \"you\", and its other branch about someone else, named by \
$gs2 or $gs3. A condition on $gn4 goes only inside the first branch: the player \
character's gender says nothing about anyone else.
- Words after such a comparison that agree with its person go inside its branches, \
since they agree differently with \"you\" and with someone else. Another player's \
gender is $gn5 for $gs2 and $gn6 for $gs3; when $gn7 or $gn8 is set, the person is a \
character or an object, female when <if \"<sheet BNpcName $gn7 6>\"> holds (or $gn8 \
for $gs3). Keep the source's branches with the name as they are and give the agreeing \
words a condition of their own. In Russian: «<if ($gs1 == $gs2)>Вы покинули<else><if \
$gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if> <if $gn7><if \"<sheet BNpcName $gn7 \
6>\">покинула<else>покинул</if><else><if $gn5>покинула<else>покинул</if></if></if> \
группу».";

/// How translations are written as the game's macro text.
pub const MACRO_TEXT: &str = "\
Macro text:
- A string's source is macro text, the game's written form; the macros each line uses \
are explained with it. Write translations as macro text and localize them: word order, \
conditions, and formatting follow the target language, not the source's shape.
- Keep every macro marked as game data; it may move or repeat. Formatting (italics, \
bold, colors) is the translation's own, as in the official localizations: add, drop, or \
move it as the target language reads best, and close every tag you open, as the source \
does. Conditions may be reworded, \
restructured, added, or dropped: add one where the target language must agree with the \
player character's gender or another known value. Write \\< \\{ \\\\ for literal \
characters.
- A name the game fills in (<sheet …>, <noun-…>, a player's name) shows as stored, in its \
base form: it never changes for case. In a language with cases, phrase the line so the \
name stands where that form is right; in Russian, the nominative: «<noun-en ObjStr 2 \
$gn7 1 1> покидает группу», «Получено: <sheet Item $n1 0>», never «Вы приглашаете \
<noun-en ObjStr 2 $gn7 1 1>».";

/// Language notes on living text for one target language, if Aeria has
/// them.
#[must_use]
pub fn living_language(target: &str) -> Option<&'static str> {
    target.eq_ignore_ascii_case("ru").then_some(LIVING_RUSSIAN)
}

/// How machine-written Russian gives itself away, for Russian targets.
const LIVING_RUSSIAN: &str = "\
Living Russian:
- Speech lives on particles, interjections, and word order: же, ведь, -то, уж, ну, вот, \
мол, дескать, небось, а, да, эх, ох, as the speaker would use them, never as filler.
- No канцелярит: является, данный, осуществлять, производить, обеспечить, соблюдать, \
предоставить, состояться, в связи с, в целях, в рамках, «регистрация завершена» where \
«теперь ты в гильдии» would do; no bookish links in speech (однако, тем не менее, кроме \
того, таким образом, в конце концов for \"after all\").
- No calques: «Если это не…» for \"If it isn't…\", «Поверь моему слову», «звучит как», \
«иметь смысл», possessives and pronouns Russian leaves out (свой, его, мой, ты, я).
- Few colons and dashes: one explanation per line at most; Russian speech prefers a new \
sentence, «а», «да», «вот и», or word order.";

/// Phrasing that makes a line read machine-written, as short descriptions;
/// empty when the text has none or the target has no list. Plain word
/// matching on the text without tags, cheap enough to run on every line.
#[must_use]
pub fn machine_phrasing(target: &str, text: &str) -> Vec<&'static str> {
    if !target.eq_ignore_ascii_case("ru") {
        return Vec::new();
    }
    let plain = strip_tags(text).to_lowercase();
    let words: Vec<&str> = plain
        .split(|c: char| !c.is_alphabetic() && c != '-')
        .filter(|word| !word.is_empty())
        .collect();
    let mut found = Vec::new();
    let officialese = [
        "является",
        "являются",
        "являлся",
        "данный",
        "данная",
        "данное",
        "данные",
        "данного",
        "данной",
    ];
    let officialese_stems = [
        "осуществ",
        "обеспеч",
        "соблюда",
        "предостав",
        "состоял",
        "состоится",
    ];
    if words.iter().any(|word| {
        officialese.contains(word) || officialese_stems.iter().any(|stem| word.starts_with(stem))
    }) || ["в связи с", "в целях", "в рамках", "таким образом"]
        .iter()
        .any(|phrase| plain.contains(phrase))
    {
        found.push("канцелярит");
    }
    if [
        "однако",
        "тем не менее",
        "кроме того",
        "в конце концов",
        "следовательно",
    ]
    .iter()
    .any(|phrase| plain.contains(phrase))
    {
        found.push("книжная связка");
    }
    if ["если это не ", "поверь моему слову", "звучит как"]
        .iter()
        .any(|phrase| plain.starts_with(phrase) || plain.contains(&format!(". {phrase}")))
    {
        found.push("калька");
    }
    if plain.matches(": ").count() + plain.matches(" — ").count() >= 2 {
        found.push("двоеточия и тире-пояснения");
    }
    if words
        .iter()
        .filter(|word| word.starts_with("котор"))
        .count()
        >= 2
    {
        found.push("два «который»");
    }
    found
}

fn strip_tags(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut depth = 0_usize;
    for c in text.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_phrasing_finds_russian_officialese_calques_and_explanations() {
        assert_eq!(
            machine_phrasing("ru", "Твоя регистрация является завершённой."),
            ["канцелярит"]
        );
        assert_eq!(
            machine_phrasing("ru", "Если это не звёздная журналистка!"),
            ["калька"]
        );
        assert_eq!(
            machine_phrasing("ru", "Напомню: тут опасно — будь начеку."),
            ["двоеточия и тире-пояснения"]
        );
        assert!(machine_phrasing("ru", "Ну что, <i>небось</i> не терпится?").is_empty());
        assert!(machine_phrasing("fr", "является").is_empty());
    }
}
