//! What a request says: instructions that are the same for every request of
//! a run, and the batch with what it needs: the game's names and the
//! project's terms that occur in it, and translated strings of the same file
//! as examples.

use std::collections::HashMap;
use std::fmt::Write as _;

use aeria_knowledge::rules::{
    MACRO_TEXT, ORIGINAL_TEXT, PLAYER_CHARACTER, TRANSLATION_STYLE, living_language,
};
use serde_json::{Value, json};

/// The instructions of every request of a run: the rules of a translation,
/// the project's style, and the answer's form.
#[must_use]
pub fn instructions(source_language: &str, target_language: &str, style: Option<&str>) -> String {
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
        "The request is JSON: `names` are the game's names that occur in the batch with the \
         project's translations, which you use exactly; `terms` are the project's terms, \
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
         translation; keep what still fits.\n\n\
         Answer with one JSON object and nothing else: each key is the `id` of a string and \
         each value its translation as macro text. Translate every string.",
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
pub fn input(files: &[FileTask], names: &[(String, String)], terms: &[Term]) -> String {
    let value = json!({
        "names": names
            .iter()
            .map(|(source, translation)| json!({ "name": source, "translation": translation }))
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
                value
            })
            .collect::<Vec<_>>(),
    })
}

/// The texts of a string whose line varies with the player character's
/// gender: `source`, and the languages of its context (`fr: …`) with a
/// condition on `$gn4`. The rule alone is easy to miss in a long request;
/// the mark sits on the string that needs it.
fn gendered(item: &Item) -> Vec<&str> {
    let mut texts = Vec::new();
    if item.source.contains("$gn4") {
        texts.push("source");
    }
    for line in &item.context {
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
         object of only these ids and their corrected translations.\n{}",
        Value::from(fixes)
    )
}

/// Reads an answer: the first JSON object in it, as ids and translations.
///
/// # Errors
///
/// Returns a description when the answer holds no JSON object of strings.
pub fn parse(text: &str) -> Result<HashMap<String, String>, String> {
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
        .filter_map(|(id, value)| value.as_str().map(|text| (id.clone(), text.to_owned())))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_from_the_first_json_object() {
        let parsed = parse("Here:\n```json\n{\"1\": \"ОК\", \"2\": \"Отмена\", \"3\": 4}\n```")
            .expect("parsed");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["1"], "ОК");
        assert!(parse("no object").is_err());
        assert!(parse("[1, 2]").is_err());
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
                }],
            }],
            &[("Minfilia".to_owned(), "Минфилия".to_owned())],
            &[],
        );
        let value: Value = serde_json::from_str(&input).expect("json");
        assert_eq!(value["files"][0]["strings"][0]["context"][0], "de: Ok");
        assert_eq!(value["names"][0]["translation"], "Минфилия");
        assert_eq!(value["files"][0]["speakers"][0]["speaker"], "MINFILIA");
        assert_eq!(value["files"][0]["about"], "Addon");
        assert!(value["files"][0]["strings"][0].get("gendered").is_none());
    }

    #[test]
    fn strings_that_vary_with_the_player_gender_are_marked() {
        let item = |source: &str, context: &[&str]| Item {
            id: "1".to_owned(),
            source: source.to_owned(),
            context: context.iter().map(|line| (*line).to_owned()).collect(),
            previous: None,
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
}
