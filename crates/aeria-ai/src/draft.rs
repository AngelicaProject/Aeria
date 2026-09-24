//! A one-string translation draft for the editor's "Draft with Angelica".
//!
//! One request without tools translates the string in tagged form. The
//! result is rebuilt and checked with the assisted structure policy; a
//! refused result is sent back with what to fix, a bounded number of times.

use std::fmt::Write as _;

use crate::chat::{ChatMessage, ChatRequest, Usage};
use crate::client::{OpenAiCompatibleClient, ProviderEndpoint, ProviderError};
use crate::provider::ReasoningEffort;
use crate::tools::{ContextCell, ProjectFacts};

/// Most requests one draft makes, including corrections.
pub const MAX_DRAFT_ATTEMPTS: usize = 3;

/// Everything the model sees for one string.
#[derive(Clone, Debug)]
pub struct DraftRequest<'a> {
    pub model: &'a str,
    pub effort: Option<ReasoningEffort>,
    /// Stable ID for providers that use a session header.
    pub session: &'a str,
    pub facts: Option<&'a ProjectFacts>,
    pub location: &'a str,
    pub source: &'a str,
    pub context: &'a [ContextCell],
    pub current_target: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// A validated draft.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Draft {
    /// The rebuilt target macro string.
    pub target: String,
    pub usage: Usage,
}

/// Why no draft was produced.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("the source string is malformed and cannot be translated with assistance")]
    Untaggable,
    #[error("the model's translation still broke the string's structure: {}", .0.join("; "))]
    Rejected(Vec<String>),
}

const DRAFT_INSTRUCTIONS: &str = "\
You are Angelica, the translation agent of Aeria, a FINAL FANTASY XIV translation tool. \
Translate one game string. Reply with the translation only, between <translation> and \
</translation>, and nothing else.

The source is given in tagged form: prose with tags such as <x id=\"1\"/> or \
<g id=\"2\"><b>…</b><b>…</b></g> for game macros, described in the legend. Keep every \
tag exactly. Tags may move within their level to fit word order, but formatting tags keep \
their order, tags inside a <b> branch stay in that branch, and only tags marked \"may \
repeat\" may repeat. Translate the text inside every <b>. Write &lt; &gt; &amp; for literal \
characters. The game cannot compute number endings, so prefer number-neutral phrasing.";

fn user_message(request: &DraftRequest<'_>, tagged: &str, legend: &[String]) -> String {
    let mut message = String::new();
    if let Some(facts) = request.facts {
        let target = facts
            .target_language
            .as_deref()
            .unwrap_or("the project's target language");
        let _ = writeln!(
            message,
            "Translate from {} into {target}.",
            facts.source_language
        );
    }
    let _ = writeln!(message, "String {}:", request.location);
    let _ = writeln!(message, "<source>{tagged}</source>");
    if !legend.is_empty() {
        message.push_str("Legend:\n");
        for line in legend {
            let _ = writeln!(message, "- {line}");
        }
    }
    if !request.context.is_empty() {
        message.push_str("Other text of the same row, for context only:\n");
        for cell in request.context {
            let _ = writeln!(message, "- column {}: {}", cell.column, cell.source);
        }
    }
    if let Some(target) = request.current_target {
        let _ = writeln!(message, "Current translation, to improve: {target}");
    }
    if let Some(note) = request.note {
        let _ = writeln!(message, "Translator note: {note}");
    }
    message
}

/// Extracts the text between `<translation>` markers, or the whole reply
/// when the model left them out.
fn extract(reply: &str) -> &str {
    let reply = reply.trim();
    match (reply.find("<translation>"), reply.rfind("</translation>")) {
        (Some(start), Some(end)) if start + "<translation>".len() <= end => {
            reply[start + "<translation>".len()..end].trim()
        }
        _ => reply,
    }
}

/// Drafts one translation.
///
/// # Errors
///
/// Returns a provider error, [`DraftError::Untaggable`] for a malformed
/// source, or [`DraftError::Rejected`] when every attempt broke the
/// structure.
pub async fn draft_translation(
    client: &OpenAiCompatibleClient,
    endpoint: &ProviderEndpoint,
    request: &DraftRequest<'_>,
) -> Result<Draft, DraftError> {
    let tagged = aeria_se::project(request.source).map_err(|_| DraftError::Untaggable)?;
    let legend: Vec<String> = tagged.tags.iter().map(aeria_se::Tag::legend).collect();
    let mut messages = vec![ChatMessage::User {
        content: user_message(request, &tagged.text, &legend),
    }];
    let mut usage = Usage::default();
    let mut last_errors = Vec::new();
    for _ in 0..MAX_DRAFT_ATTEMPTS {
        let chat = ChatRequest {
            model: request.model,
            effort: request.effort,
            system: DRAFT_INSTRUCTIONS,
            messages: &messages,
            tools: &[],
            turn_start: messages.len(),
        };
        let response = client
            .stream_chat(endpoint, request.session, &chat, &mut |_| {})
            .await?;
        if let Some(response_usage) = response.usage {
            usage.add(response_usage);
        }
        let candidate = extract(&response.content).to_owned();
        match aeria_se::rebuild(request.source, &candidate) {
            Ok(target) if !target.trim().is_empty() => return Ok(Draft { target, usage }),
            Ok(_) => last_errors = vec!["the translation is empty".to_owned()],
            Err(errors) => last_errors = errors.into_iter().map(|error| error.message).collect(),
        }
        messages.push(ChatMessage::Assistant {
            content: response.content,
            reasoning: None,
            tool_calls: Vec::new(),
        });
        messages.push(ChatMessage::User {
            content: format!(
                "That translation was refused:\n- {}\nReply with the corrected translation only, between <translation> and </translation>.",
                last_errors.join("\n- ")
            ),
        });
    }
    Err(DraftError::Rejected(last_errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_read_between_translation_markers() {
        assert_eq!(
            extract("Sure!\n<translation> Привет </translation>\nDone"),
            "Привет"
        );
        assert_eq!(extract("Привет"), "Привет");
        assert_eq!(
            extract("</translation>oops<translation>"),
            "</translation>oops<translation>"
        );
    }

    #[test]
    fn the_request_describes_tags_context_and_current_state() {
        let context = [ContextCell {
            column: 1,
            source: "Description".to_owned(),
        }];
        let request = DraftRequest {
            model: "m",
            effort: None,
            session: "s",
            facts: None,
            location: "Item:5:0:0",
            source: "Hi <pcname(lnum1)>",
            context: &context,
            current_target: Some("Привет"),
            note: Some("informal"),
        };
        let tagged = aeria_se::project(request.source).expect("tags");
        let legend: Vec<String> = tagged.tags.iter().map(aeria_se::Tag::legend).collect();
        let message = user_message(&request, &tagged.text, &legend);
        assert!(message.contains(r#"<source>Hi <x id="1"/></source>"#));
        assert!(message.contains("1: <pcname(lnum1)>"));
        assert!(message.contains("column 1: Description"));
        assert!(message.contains("Current translation, to improve: Привет"));
        assert!(message.contains("Translator note: informal"));
    }
}
