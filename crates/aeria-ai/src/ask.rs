//! `ask_choice`: one question to the user with options the panel shows as
//! buttons, and a way to answer in the user's own words.
//!
//! The tool changes nothing. The desktop shows the question and its options
//! under Angelica's reply; the user's pick, or anything the user writes
//! instead, arrives as the next message. Angelica asks one question at a
//! time and ends her turn after asking.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::chat::ToolDefinition;
use crate::tools::ToolError;

/// Options of one question, at least and at most.
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 4;
/// Longest text of a question or an option.
const MAX_TEXT_CHARS: usize = 4_000;

/// The tool, offered in every mode.
#[must_use]
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "ask_choice",
        description: "Asks the user one question with 2 to 4 options, which the panel shows as buttons, together with a way to answer in the user's own words. Use it whenever the user has to choose, such as the variants of a style calibration; ask one question at a time, and end your turn right after asking without repeating the options in your reply. The answer arrives as the next message.",
        parameters: json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "The question, with what the user needs to decide; Markdown." },
                "options": {
                    "type": "array",
                    "minItems": MIN_OPTIONS,
                    "maxItems": MAX_OPTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string", "description": "A short label, such as A; defaults to A, B, C, D." },
                            "text": { "type": "string", "description": "The option in full, such as the lines of a variant; Markdown." },
                        },
                        "required": ["text"],
                        "additionalProperties": false,
                    },
                },
            },
            "required": ["question", "options"],
            "additionalProperties": false,
        }),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionArgs {
    label: Option<String>,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskArgs {
    question: String,
    options: Vec<OptionArgs>,
}

/// Checks a question and acknowledges it.
///
/// # Errors
///
/// Returns an error for a question without text or with too few, too many,
/// empty, or repeated options.
pub fn ask_choice(arguments: &str) -> Result<Value, ToolError> {
    let args: AskArgs = crate::tools::parse(arguments)?;
    if args.question.trim().is_empty() {
        return Err(ToolError::new("the question has no text"));
    }
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&args.options.len()) {
        return Err(ToolError::new(format!(
            "give {MIN_OPTIONS} to {MAX_OPTIONS} options"
        )));
    }
    let mut labels: Vec<String> = Vec::new();
    for (index, option) in args.options.iter().enumerate() {
        let label = option
            .label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map_or_else(
                || char::from(b'A' + u8::try_from(index).unwrap_or(0)).to_string(),
                str::to_owned,
            );
        if option.text.trim().is_empty() {
            return Err(ToolError::new(format!("option {label} has no text")));
        }
        if labels.contains(&label) {
            return Err(ToolError::new(format!("option {label} is given twice")));
        }
        labels.push(label);
    }
    if args.question.chars().count() > MAX_TEXT_CHARS
        || args
            .options
            .iter()
            .any(|option| option.text.chars().count() > MAX_TEXT_CHARS)
    {
        return Err(ToolError::new(format!(
            "keep the question and each option under {MAX_TEXT_CHARS} characters"
        )));
    }
    Ok(json!({
        "status": "asked",
        "labels": labels,
        "note": "The user sees the question with its options and answers with the next message, by choosing an option or in their own words. End your turn now.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_need_text_and_distinct_options() {
        let asked = ask_choice(r#"{"question": "Which?", "options": [{"text": "one"}, {"label": "X", "text": "two"}, {"text": "three"}]}"#).expect("asked");
        assert_eq!(asked["labels"], json!(["A", "X", "C"]));
        for arguments in [
            r#"{"question": " ", "options": [{"text": "a"}, {"text": "b"}]}"#,
            r#"{"question": "Q", "options": [{"text": "a"}]}"#,
            r#"{"question": "Q", "options": [{"text": "a"}, {"text": " "}]}"#,
            r#"{"question": "Q", "options": [{"label": "A", "text": "a"}, {"label": "A", "text": "b"}]}"#,
        ] {
            assert!(ask_choice(arguments).is_err(), "{arguments}");
        }
    }
}
