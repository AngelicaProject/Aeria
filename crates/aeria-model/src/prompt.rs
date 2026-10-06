//! What a request says: instructions that are the same for every request of
//! a run, and the batch with what it needs: the game's names and the
//! project's terms that occur in it, and translated strings of the same file
//! as examples.

use std::collections::HashMap;
use std::fmt::Write as _;

use aeria_knowledge::rules::{
    MACRO_TEXT, ORIGINAL_TEXT, PLAYER_CHARACTER, TRANSLATION_STYLE, living_language,
};
use aeria_po::length::{Budget, Unit};
use serde_json::{Value, json};

use crate::names::Name;

/// The instructions of every request of a run: the rules of a translation,
/// the project's style, and the answer's form.
#[must_use]
pub fn instructions(source_language: &str, target_language: &str, style: Option<&str>) -> String {
    let mut text = rules(source_language, target_language, style);
    text.push_str(
        "Answer with one JSON object and nothing else: each key is the `id` of a string and \
         each value an array of two strings: the first words of that string's source, up to \
         three, copied as they are (macros may be left out), then its translation as macro \
         text. The first words show which string a translation belongs to; give every \
         string its own translation. Translate every string.",
    );
    text
}

/// The instructions of every request of a run that corrects translations:
/// the rules and the style as for translating, what a correction may
/// change, a proofreading of every translation when asked, a translator's
/// request for every translation of the run, and the answer's form.
#[must_use]
pub fn fix_instructions(
    source_language: &str,
    target_language: &str,
    style: Option<&str>,
    proofread: bool,
    adapt: bool,
    request: Option<&str>,
) -> String {
    let mut text = rules(source_language, target_language, style);
    text.push_str(
        "This run corrects translations rather than translating: each string has \
         `translation`, its current translation, and may have `fix`: what the project's \
         checks find wrong with it. Change a translation only as far as its `fix`, the \
         proofreading below when this run asks for it, and the translator's request below \
         need, and keep every other word, its macros, and its wording as they are. Advice \
         of the checks can be wrong for a string: when an item of `fix` does not apply, or \
         nothing needs to change, answer the translation unchanged.\n\n",
    );
    if proofread {
        text.push_str(PROOFREADING);
    }
    if adapt {
        text.push_str(ADAPTING);
    }
    if let Some(request) = request.map(str::trim).filter(|request| !request.is_empty()) {
        let _ = write!(
            text,
            "The translator asks of every translation of this run:\n\n{request}\n\n"
        );
    }
    let _ = write!(
        text,
        "Answer with one JSON object and nothing else: each key is the `id` of a string and \
         each value an array of three strings: the first words of that string's source, up \
         to three, copied as they are (macros may be left out); then its corrected \
         translation as macro text; then why it changed, in a few words of \
         {target_language} without quotes (обращение по французской строке, канцелярит, \
         калька, потерян смысл источника), or an empty string when it did not. The first \
         words show which string a translation belongs to; give every string its own \
         translation. Answer every string."
    );
    text
}

/// What a correction run asks of a translation whose source changed.
const ADAPTING: &str = "A string with `previous` changed its source since it was translated: \
     `previous.source` is the source its `translation` was written for, and `source` the \
     source now. Adapt the translation to the new source: change what the change of the \
     source changes, such as a number, a name, a word, or a sentence added or removed, and \
     keep every other word of the translation as it is. When the change of the source does \
     not touch what the translation says, answer it unchanged: it still fits.\n\n";

/// What proofreading asks of every translation of a correction run.
const PROOFREADING: &str = "Proofread every translation of this run as the editor of the \
     localization, reading it against its source, the other client languages of its \
     context, and the strings around it in the scene. Fix what is wrong: a meaning the \
     source does not have, or one of its details lost; the form of address the project's \
     style asks for in this line, which when the style follows another client language is \
     read from that language's line of this string, not from the lines around it, since \
     characters address each other differently; agreement with the player character's \
     gender; a name or term; and phrasing a native speaker would not write: word order or \
     constructions copied from the source language, officialese, filler, explanations the \
     source does not give. The source's meaning comes first: never trade a detail of it \
     for smoothness, never add what it does not say, and keep the register and the voice \
     of the speaker. Keep what is right as it is: a translation that already reads well \
     answers unchanged, and a sound change is the smallest one that fixes it. A name of a \
     thing of the game (an action, a status, an item, a place, a duty, a menu, a tab, a \
     button) keeps the form the translation gives it unless `names` or `terms` give \
     another: the game shows it under that name, and a player looks for it there.\n\n";

/// What the instructions of a translation and of a correction share: the
/// rules of a translation, the project's style, and what a request holds.
fn rules(source_language: &str, target_language: &str, style: Option<&str>) -> String {
    let mut text = format!(
        "You translate the text of FINAL FANTASY XIV from {source_language} into \
         {target_language} for a fan localization. Each request is a batch of strings of one \
         or more files of the game, each file's strings in its order; strings of a scene are \
         lines of one dialogue, and files never share a dialogue.\n\n"
    );
    for section in [ORIGINAL_TEXT, TRANSLATION_STYLE, PLAYER_CHARACTER] {
        text.push_str(section);
        text.push_str("\n\n");
    }
    if let Some(living) = living_language(target_language) {
        text.push_str(living);
        text.push_str("\n\n");
    }
    text.push_str(MACRO_TEXT);
    text.push('\n');
    text.push_str(&aeria_se::authoring_reference());
    text.push_str("\n\n");
    if let Some(style) = style.map(str::trim).filter(|style| !style.is_empty()) {
        let _ = write!(
            text,
            "The project's style, decided by its translators, takes precedence over the \
             defaults above:\n\n{style}\n\n"
        );
    }
    text.push_str(
        "The request is JSON: `names` are the game's names found in the batch, each with the \
         project's translation and `from`: what it names and the sheet of the game it comes \
         from. A name is found by its letters alone, so a word that only looks like one is \
         listed too: `Walk` in \"A Walk in the Park\" is not the place \"The Walk\". Where a \
         word of a string is that name, you use its translation, in the form the sentence \
         needs: the translation is the name as it stands alone, and in a language that \
         declines names it declines like any written word (Поговорите с Ко Рабнтой); only a \
         name the game fills in through a macro keeps its stored form. Where the word \
         means something else, you translate it by its meaning. `terms` are the project's \
         terms, \
         which you use exactly and whose `never` variants you never use; `files` are the \
         files of the batch, each on its own: `about` says what the file is: its sheet, a \
         quest's title, and whether its strings are in play order, so that a batch continues \
         the strings before it; `speakers` are the characters who speak in the file by their \
         labels (the `speaker` of a string's context) with their names and the project's \
         translations; `examples` are translated strings of the same file, whose wording you \
         continue; `strings` are the strings to translate, each with its `id`, its `source`, \
         and `context`: the other client languages (ja, de, fr), the speaker or kind of a \
         line, other fields of its row, and what its macros do. A string with `gendered` \
         names the texts (`source`, or a language of `context`) whose line varies with the \
         player character's gender: there the translation needs a condition on $gn4 for \
         every word that agrees with the player character, unless it is phrased so that \
         nothing does. A string with `previous` was translated before its \
         source changed: `previous.source` is the old source and `previous.translation` its \
         translation; keep what still fits. A string with `termExceptions` names terms of \
         `terms` that do not apply to it, as a person decided: there the word means something \
         else, so you translate it by its meaning and may use the `never` variants. A string \
         with `maxLength` is an interface label: \
         its translation shows at most that many characters (macros not counted), as the \
         official localizations fit the game's layout; shorten it, with the usual \
         abbreviations of the language when needed (Шанс прям. удара). A string with \
         `maxBytes` is the name of a character or object the game shows over it in the \
         world: the game cuts a name longer than that many bytes of UTF-8, where a Latin \
         letter, digit, or space takes one and a Cyrillic letter or « » two; keep the name \
         within it, shortening a description or using the usual abbreviations of the \
         language when needed.\n\n",
    );
    text
}

/// A string of a batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub id: String,
    pub source: String,
    pub context: Vec<String>,
    /// The old source and translation of a fuzzy string.
    pub previous: Option<(String, String)>,
    /// The longest an interface label's translation may show, or a world
    /// object's name may be.
    pub max_length: Option<Budget>,
    /// Terms a person decided do not apply to the string.
    pub term_exceptions: Vec<String>,
    /// The translation a correction starts from.
    pub translation: Option<String>,
    /// What the checks find wrong with `translation`, as the model is told.
    pub fix: Vec<String>,
}

/// One answer: the words it repeats from the start of its source, and the
/// translation. `start` is `None` when the answer gave a bare translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Answer {
    pub start: Option<String>,
    pub text: String,
    /// Why a correction changed the translation, as the model says.
    pub reason: Option<String>,
}

/// A term of the project for a batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Term {
    pub term: String,
    pub translation: String,
    pub note: Option<String>,
    pub never: Vec<String>,
}

/// One file of a request: what it is, who speaks, translated strings as
/// examples, and the strings to translate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileTask {
    /// The file, relative to the project root: `po/...`.
    pub file: String,
    pub about: String,
    /// Speaker labels with their names and translations.
    pub speakers: Vec<(String, String, String)>,
    pub examples: Vec<(String, String)>,
    pub items: Vec<Item>,
}

/// The task of one request.
#[must_use]
pub fn input(files: &[FileTask], names: &[Name], terms: &[Term]) -> String {
    let value = json!({
        "names": names
            .iter()
            .map(|name| {
                let mut value = json!({ "name": name.source, "translation": name.translation });
                if let Some(origin) = name.origin() {
                    value["from"] = Value::from(origin);
                }
                value
            })
            .collect::<Vec<_>>(),
        "terms": terms
            .iter()
            .map(|term| {
                let mut value = json!({ "term": term.term, "translation": term.translation });
                if let Some(note) = &term.note {
                    value["note"] = Value::from(note.as_str());
                }
                if !term.never.is_empty() {
                    value["never"] = Value::from(term.never.clone());
                }
                value
            })
            .collect::<Vec<_>>(),
        "files": files.iter().map(file_value).collect::<Vec<_>>(),
    });
    value.to_string()
}

fn file_value(task: &FileTask) -> Value {
    json!({
        "file": task.file,
        "about": task.about,
        "speakers": task
            .speakers
            .iter()
            .map(|(label, name, translation)| json!({ "speaker": label, "name": name, "translation": translation }))
            .collect::<Vec<_>>(),
        "examples": task
            .examples
            .iter()
            .map(|(source, translation)| json!({ "source": source, "translation": translation }))
            .collect::<Vec<_>>(),
        "strings": task
            .items
            .iter()
            .map(|item| {
                let mut value = json!({ "id": item.id, "source": item.source, "context": item.context });
                let gendered = gendered(item);
                if !gendered.is_empty() {
                    value["gendered"] = Value::from(gendered);
                }
                if let Some((source, translation)) = &item.previous {
                    value["previous"] = json!({ "source": source, "translation": translation });
                }
                if let Some(budget) = item.max_length {
                    let key = match budget.unit {
                        Unit::Characters => "maxLength",
                        Unit::Bytes => "maxBytes",
                    };
                    value[key] = Value::from(budget.max);
                }
                if !item.term_exceptions.is_empty() {
                    value["termExceptions"] = Value::from(item.term_exceptions.clone());
                }
                if let Some(translation) = &item.translation {
                    value["translation"] = Value::from(translation.as_str());
                }
                if !item.fix.is_empty() {
                    value["fix"] = Value::from(item.fix.clone());
                }
                value
            })
            .collect::<Vec<_>>(),
    })
}

/// The texts of a string whose line varies with the player character's
/// gender (see [`crate::hints::gendered`]). The rule alone is easy to miss
/// in a long request; the mark sits on the string that needs it.
fn gendered(item: &Item) -> Vec<&str> {
    crate::hints::gendered(&item.source, &item.context)
}

/// A request that sends back the translations that failed the checks, with
/// what is wrong with each.
#[must_use]
pub fn retry_input(original: &str, problems: &[(String, String, Vec<String>)]) -> String {
    let fixes: Vec<Value> = problems
        .iter()
        .map(|(id, translation, reasons)| {
            json!({ "id": id, "translation": translation, "problems": reasons })
        })
        .collect();
    format!(
        "{original}\n\nThese translations of the batch have problems. Answer with one JSON \
         object of only these ids, each with the first words of its source and its corrected \
         translation, as before.\n{}",
        Value::from(fixes)
    )
}

/// Reads an answer: the first JSON object in it, as ids with the first words
/// of their source and their translations. A bare string is read as a
/// translation without its first words; other values are left out.
///
/// # Errors
///
/// Returns a description when the answer holds no JSON object.
pub fn parse(text: &str) -> Result<HashMap<String, Answer>, String> {
    let start = text.find('{').ok_or("the answer holds no JSON object")?;
    let end = text.rfind('}').ok_or("the answer holds no JSON object")?;
    if end < start {
        return Err("the answer holds no JSON object".to_owned());
    }
    let value: Value = serde_json::from_str(&text[start..=end])
        .map_err(|error| format!("the answer is not JSON: {error}"))?;
    let object = value.as_object().ok_or("the answer is not a JSON object")?;
    Ok(object
        .iter()
        .filter_map(|(id, value)| {
            let answer = match value {
                Value::String(text) => Answer {
                    start: None,
                    text: text.clone(),
                    reason: None,
                },
                Value::Array(parts) => match parts.as_slice() {
                    [Value::String(start), Value::String(text)] => Answer {
                        start: Some(start.clone()),
                        text: text.clone(),
                        reason: None,
                    },
                    [
                        Value::String(start),
                        Value::String(text),
                        Value::String(reason),
                    ] => Answer {
                        start: Some(start.clone()),
                        text: text.clone(),
                        reason: Some(reason.trim().to_owned()).filter(|reason| !reason.is_empty()),
                    },
                    _ => return None,
                },
                _ => return None,
            };
            Some((id.clone(), answer))
        })
        .collect())
}

/// Reads an answer as [`parse`] does, and when it is not valid JSON, takes
/// what can still be read: each entry `"id": ["first words", "translation"]`
/// on its own, where a quote the model did not escape inside a string is read
/// as part of it. With `reasons`, an entry ends with the reason a
/// correction gives, which has no quotes and no macros, after the last
/// `", "`. A string read this way is checked like any other, so a wrong
/// reading is refused, never written. Entries that cannot be read are left
/// out, and the caller asks for them again.
#[must_use]
pub fn parse_lenient(text: &str, reasons: bool) -> HashMap<String, Answer> {
    parse(text).unwrap_or_else(|_| salvage(text, reasons))
}

/// Where each entry `"id": [` of an answer starts: its id and the byte
/// after its `[`. Ids are the decimal numbers a request gives its strings.
fn entry_starts(text: &str) -> Vec<(String, usize)> {
    let bytes = text.as_bytes();
    let mut starts = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let digits = bytes[index + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let close = index + 1 + digits;
        if digits == 0 || bytes.get(close) != Some(&b'"') {
            index += 1;
            continue;
        }
        let mut at = close + 1;
        let skip = |at: &mut usize| {
            while bytes.get(*at).is_some_and(u8::is_ascii_whitespace) {
                *at += 1;
            }
        };
        skip(&mut at);
        if bytes.get(at) != Some(&b':') {
            index = close;
            continue;
        }
        at += 1;
        skip(&mut at);
        if bytes.get(at) != Some(&b'[') {
            index = close;
            continue;
        }
        starts.push((text[index + 1..close].to_owned(), at + 1));
        index = at + 1;
    }
    starts
}

/// The text of a JSON string's content as the model wrote it, with quotes it
/// did not escape kept as quotes; `None` when an escape is invalid.
fn unescape_loosely(raw: &str) -> Option<String> {
    let mut json = String::with_capacity(raw.len() + 2);
    json.push('"');
    let mut characters = raw.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                json.push('\\');
                json.push(characters.next()?);
            }
            '"' => json.push_str("\\\""),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            other => json.push(other),
        }
    }
    json.push('"');
    serde_json::from_str(&json).ok()
}

/// Whether `rest` starts with whitespace, then `then`.
fn followed_by(rest: &str, then: char) -> bool {
    rest.trim_start().starts_with(then)
}

/// The entries of an answer that is not valid JSON (see [`parse_lenient`]).
fn salvage(text: &str, reasons: bool) -> HashMap<String, Answer> {
    let starts = entry_starts(text);
    let mut answers = HashMap::new();
    for (position, (id, from)) in starts.iter().enumerate() {
        let to = starts
            .get(position + 1)
            .map_or(text.len(), |(_, next)| *next);
        let region = &text[*from..to];
        let Some(open) = region
            .find('"')
            .filter(|open| region[..*open].trim().is_empty())
        else {
            continue;
        };
        let body = &region[open + 1..];
        // The first words end at the first quote followed by `, "`.
        let Some(first_end) = body.match_indices('"').map(|(at, _)| at).find(|at| {
            let rest = &body[at + 1..];
            followed_by(rest, ',') && followed_by(&rest.trim_start()[1..], '"')
        }) else {
            continue;
        };
        let after = body[first_end + 1..].trim_start()[1..].trim_start();
        let Some(second) = after.strip_prefix('"') else {
            continue;
        };
        // The translation ends at the last quote followed by `]`.
        let Some(second_end) = second
            .rmatch_indices('"')
            .map(|(at, _)| at)
            .find(|at| followed_by(&second[at + 1..], ']'))
        else {
            continue;
        };
        let mut rest = &second[..second_end];
        let mut reason = None;
        if reasons
            && let Some(split) = rest.rfind("\", \"")
            && !rest[split + 4..].contains(['"', '<'])
        {
            reason = Some(rest[split + 4..].trim().to_owned()).filter(|reason| !reason.is_empty());
            rest = &rest[..split];
        }
        if let (Some(start), Some(translation)) =
            (unescape_loosely(&body[..first_end]), unescape_loosely(rest))
        {
            answers.insert(
                id.clone(),
                Answer {
                    start: Some(start),
                    text: translation,
                    reason,
                },
            );
        }
    }
    answers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_with_an_unescaped_quote_keeps_its_readable_entries() {
        let text = r#"{"1": ["Talk to", "Поговорите с <sheet Item "x" 1>."], "2": ["Hello", "Привет"], "3": ["Broken", "Сло\qмано"], "4": ["Bye", "Пока, "друг"!"]}"#;
        assert!(parse(text).is_err());
        let answers = parse_lenient(text, false);
        assert_eq!(answers["1"].text, r#"Поговорите с <sheet Item "x" 1>."#);
        assert_eq!(answers["1"].start.as_deref(), Some("Talk to"));
        assert_eq!(answers["2"].text, "Привет");
        assert!(!answers.contains_key("3"), "an invalid escape is left out");
        assert_eq!(answers["4"].text, r#"Пока, "друг"!"#);
        // Valid JSON is read as before.
        assert_eq!(parse_lenient(r#"{"1": ["a", "б"]}"#, false)["1"].text, "б");
        assert!(parse_lenient("no json", false).is_empty());
    }

    #[test]
    fn answers_are_read_from_the_first_json_object() {
        let parsed = parse(
            "Here:\n```json\n{\"1\": [\"OK\", \"ОК\"], \"2\": \"Отмена\", \"3\": 4, \"4\": [\"a\"]}\n```",
        )
        .expect("parsed");
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed["1"],
            Answer {
                start: Some("OK".to_owned()),
                text: "ОК".to_owned(),
                reason: None,
            }
        );
        assert_eq!(parsed["2"].start, None);
        assert_eq!(parsed["2"].text, "Отмена");
        assert!(parse("no object").is_err());
        assert!(parse("[1, 2]").is_err());
    }

    #[test]
    fn a_correction_gives_its_reason() {
        let valid = r#"{"1": ["OK", "Ладно", "канцелярит"], "2": ["No", "Нет", ""]}"#;
        let parsed = parse_lenient(valid, true);
        assert_eq!(parsed["1"].text, "Ладно");
        assert_eq!(parsed["1"].reason.as_deref(), Some("канцелярит"));
        assert_eq!(parsed["2"].reason, None);
        // An answer that is not JSON keeps its reasons apart from the text.
        let broken = r#"{"1": ["Talk to", "Поговорите с <sheet Item "x" 1>.", "калька"], "2": ["Hi", "Привет, "друг""]}"#;
        let salvaged = parse_lenient(broken, true);
        assert_eq!(salvaged["1"].text, r#"Поговорите с <sheet Item "x" 1>."#);
        assert_eq!(salvaged["1"].reason.as_deref(), Some("калька"));
        assert_eq!(salvaged["2"].text, r#"Привет, "друг""#);
        assert_eq!(salvaged["2"].reason, None);
    }

    #[test]
    fn instructions_carry_the_rules_and_the_style() {
        let text = instructions("en", "ru", Some("К герою на «вы»."));
        assert!(text.contains("FINAL FANTASY XIV is written in Japanese"));
        assert!(text.contains("Living Russian"));
        assert!(text.contains("К герою на «вы»."));
        assert!(!instructions("en", "fr", None).contains("Living Russian"));
        let input = input(
            &[FileTask {
                file: "po/Addon/0.po".to_owned(),
                about: "Addon".to_owned(),
                speakers: vec![(
                    "MINFILIA".to_owned(),
                    "Minfilia".to_owned(),
                    "Минфилия".to_owned(),
                )],
                examples: Vec::new(),
                items: vec![Item {
                    id: "1".to_owned(),
                    source: "OK".to_owned(),
                    context: vec!["de: Ok".to_owned()],
                    previous: None,
                    max_length: None,
                    term_exceptions: vec!["Maelstrom".to_owned()],
                    translation: None,
                    fix: Vec::new(),
                }],
            }],
            &[
                Name::new("Minfilia", "Минфилия"),
                Name {
                    context: "PlaceName:1861:0:2".to_owned(),
                    full: Some("The Walk".to_owned()),
                    ..Name::new("Walk", "переход")
                },
            ],
            &[],
        );
        let value: Value = serde_json::from_str(&input).expect("json");
        assert_eq!(value["files"][0]["strings"][0]["context"][0], "de: Ok");
        assert_eq!(value["names"][0]["translation"], "Минфилия");
        assert!(value["names"][0].get("from").is_none());
        assert_eq!(
            value["names"][1]["from"],
            "the name of a place (sheet PlaceName), a form of \"The Walk\""
        );
        assert_eq!(value["files"][0]["speakers"][0]["speaker"], "MINFILIA");
        assert_eq!(value["files"][0]["about"], "Addon");
        assert!(value["files"][0]["strings"][0].get("gendered").is_none());
        assert_eq!(
            value["files"][0]["strings"][0]["termExceptions"][0],
            "Maelstrom"
        );
        assert!(text.contains("`termExceptions`"));
    }

    #[test]
    fn a_correction_carries_the_rules_and_the_request() {
        let text = fix_instructions(
            "en",
            "ru",
            Some("К герою на «вы»."),
            true,
            true,
            Some(" На «ты». "),
        );
        assert!(text.contains("FINAL FANTASY XIV is written in Japanese"));
        assert!(text.contains("К герою на «вы»."));
        assert!(text.contains("`fix`"));
        assert!(text.contains("of this run:\n\nНа «ты».\n\n"));
        assert!(text.contains("Proofread every translation"));
        assert!(text.contains("Adapt the translation to the new source"));
        let plain = fix_instructions("en", "ru", None, false, false, None);
        assert!(!plain.contains("Proofread every") && !plain.contains("Adapt the translation"));
        assert!(!text.contains("Translate every string."));
        assert!(
            !fix_instructions("en", "ru", None, false, false, Some("  ")).contains("of this run")
        );
    }

    #[test]
    fn strings_that_vary_with_the_player_gender_are_marked() {
        let item = |source: &str, context: &[&str]| Item {
            id: "1".to_owned(),
            source: source.to_owned(),
            context: context.iter().map(|line| (*line).to_owned()).collect(),
            previous: None,
            max_length: None,
            term_exceptions: Vec::new(),
            translation: None,
            fix: Vec::new(),
        };
        let input = input(
            &[FileTask {
                items: vec![
                    item(
                        "Here to train as well, are you?",
                        &[
                            "ja: あなたも訓練？",
                            "fr: Tu es <if $gn4>venue<else>venu</if> t'entraîner ?",
                            "speaker: MAN",
                        ],
                    ),
                    item("<if $gn4>Lady<else>Sir</if>!", &["de: Hallo!"]),
                    item(
                        "Well met.",
                        &["fr: Salut.", "note: $gn4 is the player's gender"],
                    ),
                ],
                ..FileTask::default()
            }],
            &[],
            &[],
        );
        let value: Value = serde_json::from_str(&input).expect("json");
        let strings = &value["files"][0]["strings"];
        assert_eq!(strings[0]["gendered"], json!(["fr"]));
        assert_eq!(strings[1]["gendered"], json!(["source"]));
        assert!(strings[2].get("gendered").is_none());
    }

    #[test]
    fn a_length_goes_in_its_unit() {
        let item = |max_length| Item {
            id: "1".to_owned(),
            source: "aide".to_owned(),
            context: Vec::new(),
            previous: None,
            max_length,
            term_exceptions: Vec::new(),
            translation: None,
            fix: Vec::new(),
        };
        let input = input(
            &[FileTask {
                items: vec![
                    item(Some(Budget {
                        max: 8,
                        unit: Unit::Characters,
                    })),
                    item(Some(Budget {
                        max: 63,
                        unit: Unit::Bytes,
                    })),
                ],
                ..FileTask::default()
            }],
            &[],
            &[],
        );
        let value: Value = serde_json::from_str(&input).expect("json");
        let strings = &value["files"][0]["strings"];
        assert_eq!(strings[0]["maxLength"], 8);
        assert!(strings[0].get("maxBytes").is_none());
        assert_eq!(strings[1]["maxBytes"], 63);
        assert!(strings[1].get("maxLength").is_none());
        assert!(instructions("en", "ru", None).contains("`maxBytes`"));
    }
}
