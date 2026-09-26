//! Angelica's conversation loop: stream a response, run requested tools,
//! and repeat until the model answers without tools.

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;

use serde::Serialize;

use crate::chat::{ChatMessage, ChatRequest, StreamDelta, ToolCall, ToolDefinition, Usage};
use crate::client::{OpenAiCompatibleClient, ProviderEndpoint, ProviderError};
use crate::images::ImagePayloads;
use crate::provider::ReasoningEffort;
use crate::tools::ToolOutput;

/// Most model responses in one user turn.
pub const MAX_ROUNDS_PER_TURN: usize = 24;
/// Context window assumed when the model's is unknown.
pub const DEFAULT_CONTEXT_TOKENS: u32 = 64_000;
/// Share of the context window the request may use.
const CONTEXT_SHARE: f64 = 0.75;
/// Conservative characters-per-token estimate for budgeting.
const CHARS_PER_TOKEN: f64 = 3.0;
const OMITTED_TOOL_RESULT: &str = "[earlier tool result omitted to save context]";

/// Runs tool calls, possibly on another thread.
pub trait ToolExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        call: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = ToolOutput> + Send + 'a>>;
}

/// Progress of a turn, for live presentation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum AgentEvent {
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    /// Tool-call arguments received while a response streams. The text
    /// serves live progress only and is not sent to the renderer.
    ToolArgumentsDelta {
        chars: usize,
        #[serde(skip)]
        text: String,
    },
    /// A response finished streaming.
    ResponseFinished,
    ToolStarted {
        id: String,
        name: String,
        arguments: String,
    },
    ToolFinished {
        id: String,
        name: String,
        content: String,
        is_error: bool,
    },
    Usage {
        #[serde(flatten)]
        usage: Usage,
    },
}

/// Model settings for one turn.
#[derive(Clone, Debug)]
pub struct TurnConfig<'a> {
    pub model: &'a str,
    pub effort: Option<ReasoningEffort>,
    pub system: &'a str,
    pub tools: &'a [ToolDefinition],
    pub context_tokens: Option<u32>,
    /// Stable per-conversation session ID for providers that use one.
    pub session: &'a str,
    /// Most model responses in the turn.
    pub max_rounds: usize,
    /// Image data when the model accepts images; `None` sends none.
    pub images: Option<&'a ImagePayloads>,
}

/// How a turn ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnOutcome {
    Completed,
    /// The model kept calling tools past [`MAX_ROUNDS_PER_TURN`].
    RoundLimit,
}

/// The result of a finished turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TurnSummary {
    pub outcome: TurnOutcome,
    pub usage: Usage,
}

/// Runs one user turn. `messages` must end with the user's message; the
/// assistant responses and tool results are appended to it, and `persist`
/// is called after each append so an interrupted turn keeps its progress.
///
/// # Errors
///
/// Returns the provider error that stopped the turn. Messages appended
/// before the failure remain in `messages`.
pub async fn run_turn(
    client: &OpenAiCompatibleClient,
    endpoint: &ProviderEndpoint,
    config: &TurnConfig<'_>,
    messages: &mut Vec<ChatMessage>,
    executor: &dyn ToolExecutor,
    on_event: &mut (dyn FnMut(AgentEvent) + Send),
    persist: &mut (dyn FnMut(&[ChatMessage]) + Send),
) -> Result<TurnSummary, ProviderError> {
    repair_dangling_tool_calls(messages);
    let turn_start = messages
        .iter()
        .rposition(|message| matches!(message, ChatMessage::User { .. }))
        .unwrap_or(0);
    let mut usage = Usage::default();
    for _ in 0..config.max_rounds.clamp(1, MAX_ROUNDS_PER_TURN) {
        let (context, context_turn_start) =
            fit_context(messages, turn_start, config.context_tokens);
        let request = ChatRequest {
            model: config.model,
            effort: config.effort,
            system: config.system,
            messages: &context,
            tools: config.tools,
            turn_start: context_turn_start,
            images: config.images,
        };
        let response = {
            let mut forward = |delta: StreamDelta| {
                on_event(match delta {
                    StreamDelta::Text(text) => AgentEvent::TextDelta { text },
                    StreamDelta::Reasoning(text) => AgentEvent::ReasoningDelta { text },
                    StreamDelta::ToolArguments(text) => AgentEvent::ToolArgumentsDelta {
                        chars: text.chars().count(),
                        text,
                    },
                });
            };
            client
                .stream_chat(endpoint, config.session, &request, &mut forward)
                .await?
        };
        on_event(AgentEvent::ResponseFinished);
        if let Some(response_usage) = response.usage {
            usage.add(response_usage);
            on_event(AgentEvent::Usage {
                usage: response_usage,
            });
        }
        let calls = response.tool_calls.clone();
        messages.push(ChatMessage::Assistant {
            content: response.content,
            reasoning: response.reasoning,
            tool_calls: response.tool_calls,
        });
        persist(messages);
        if calls.is_empty() {
            return Ok(TurnSummary {
                outcome: TurnOutcome::Completed,
                usage,
            });
        }
        for call in &calls {
            on_event(AgentEvent::ToolStarted {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });
            let output = executor.execute(call).await;
            on_event(AgentEvent::ToolFinished {
                id: call.id.clone(),
                name: call.name.clone(),
                content: output.content.clone(),
                is_error: output.is_error,
            });
            messages.push(ChatMessage::Tool {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: output.content,
            });
            persist(messages);
        }
    }
    Ok(TurnSummary {
        outcome: TurnOutcome::RoundLimit,
        usage,
    })
}

/// Adds a cancellation result for every tool call that has none, so an
/// interrupted turn never leaves the history in a shape providers reject.
pub fn repair_dangling_tool_calls(messages: &mut Vec<ChatMessage>) {
    let mut index = 0;
    while index < messages.len() {
        if let ChatMessage::Assistant { tool_calls, .. } = &messages[index]
            && !tool_calls.is_empty()
        {
            let calls = tool_calls.clone();
            let mut next = index + 1;
            let mut answered = Vec::new();
            while let Some(ChatMessage::Tool { tool_call_id, .. }) = messages.get(next) {
                answered.push(tool_call_id.clone());
                next += 1;
            }
            let missing: Vec<ChatMessage> = calls
                .iter()
                .filter(|call| !answered.contains(&call.id))
                .map(|call| ChatMessage::Tool {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    content: r#"{"error":"the tool call was cancelled"}"#.to_owned(),
                })
                .collect();
            let inserted = missing.len();
            messages.splice(next..next, missing);
            index = next + inserted;
        } else {
            index += 1;
        }
    }
}

/// Returns the messages to send within the context budget and the index at
/// which the current turn starts in them.
///
/// Earlier turns' tool results are replaced by a short notice first, then
/// their images; if that is not enough, whole earlier turns are dropped,
/// oldest first. The current turn is always sent in full.
#[must_use]
pub fn fit_context(
    messages: &[ChatMessage],
    turn_start: usize,
    context_tokens: Option<u32>,
) -> (Vec<ChatMessage>, usize) {
    let tokens = f64::from(context_tokens.unwrap_or(DEFAULT_CONTEXT_TOKENS));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let budget = (tokens * CONTEXT_SHARE * CHARS_PER_TOKEN) as usize;
    let mut context = messages.to_vec();
    let mut turn_start = turn_start.min(context.len());
    if size(&context) <= budget {
        return (context, turn_start);
    }
    for message in &mut context[..turn_start] {
        if let ChatMessage::Tool { content, .. } = message {
            OMITTED_TOOL_RESULT.clone_into(content);
        }
    }
    if size(&context) <= budget {
        return (context, turn_start);
    }
    for message in &mut context[..turn_start] {
        if let ChatMessage::User {
            content, images, ..
        } = message
            && !images.is_empty()
        {
            let _ = write!(
                content,
                "\n\n[{} earlier image(s) omitted to save context]",
                images.len()
            );
            images.clear();
        }
    }
    while size(&context) > budget && turn_start > 0 {
        // Drop the oldest whole turn: its user message and every reply.
        let end = context[1..turn_start]
            .iter()
            .position(|message| matches!(message, ChatMessage::User { .. }))
            .map_or(turn_start, |position| position + 1);
        context.drain(..end);
        turn_start -= end;
    }
    (context, turn_start)
}

fn size(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|message| match message {
            ChatMessage::User {
                content, images, ..
            } => {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_precision_loss,
                    clippy::cast_sign_loss
                )]
                let image_chars = images
                    .iter()
                    .map(|image| (image.estimated_tokens() as f64 * CHARS_PER_TOKEN) as usize)
                    .sum::<usize>();
                content.len() + image_chars
            }
            ChatMessage::Tool { content, .. } => content.len(),
            ChatMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                content.len()
                    + tool_calls
                        .iter()
                        .map(|call| call.name.len() + call.arguments.len())
                        .sum::<usize>()
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;

    use super::*;
    use crate::provider::BaseUrl;
    use crate::secrets::ApiKey;

    fn user(text: &str) -> ChatMessage {
        ChatMessage::User {
            content: text.to_owned(),
            automatic: false,
            images: Vec::new(),
        }
    }

    fn assistant_calling(id: &str) -> ChatMessage {
        ChatMessage::Assistant {
            content: String::new(),
            reasoning: None,
            tool_calls: vec![ToolCall {
                id: id.to_owned(),
                name: "list_sheets".to_owned(),
                arguments: "{}".to_owned(),
            }],
        }
    }

    fn tool(id: &str, content: &str) -> ChatMessage {
        ChatMessage::Tool {
            tool_call_id: id.to_owned(),
            name: "list_sheets".to_owned(),
            content: content.to_owned(),
        }
    }

    #[test]
    fn stored_messages_and_events_use_camel_case_fields() {
        let message = serde_json::to_value(tool("c1", "[]")).expect("message");
        assert_eq!(message["role"], "tool");
        assert_eq!(message["toolCallId"], "c1");
        let message = serde_json::to_value(assistant_calling("c1")).expect("message");
        assert_eq!(message["toolCalls"][0]["id"], "c1");
        let event = serde_json::to_value(AgentEvent::ToolFinished {
            id: "c1".to_owned(),
            name: "list_sheets".to_owned(),
            content: "[]".to_owned(),
            is_error: true,
        })
        .expect("event");
        assert_eq!(event["type"], "toolFinished");
        assert_eq!(event["isError"], true);
        let usage = serde_json::to_value(AgentEvent::Usage {
            usage: Usage {
                prompt_tokens: 1,
                completion_tokens: 2,
            },
        })
        .expect("usage");
        assert_eq!(usage["promptTokens"], 1);
    }

    #[test]
    fn dangling_tool_calls_get_cancellation_results() {
        let mut messages = vec![user("q"), assistant_calling("c1"), user("next")];
        repair_dangling_tool_calls(&mut messages);
        assert_eq!(messages.len(), 4);
        assert!(
            matches!(&messages[2], ChatMessage::Tool { tool_call_id, content, .. } if tool_call_id == "c1" && content.contains("cancelled"))
        );

        let mut complete = vec![user("q"), assistant_calling("c1"), tool("c1", "[]")];
        repair_dangling_tool_calls(&mut complete);
        assert_eq!(complete.len(), 3);
    }

    #[test]
    fn context_elides_old_tool_results_then_drops_old_turns() {
        let big = "x".repeat(40_000);
        let messages = vec![
            user("first"),
            assistant_calling("c1"),
            tool("c1", &big),
            user("second"),
            assistant_calling("c2"),
            tool("c2", "current"),
        ];
        let (context, start) = fit_context(&messages, 3, Some(8_000));
        assert_eq!(start, 3);
        assert!(
            matches!(&context[2], ChatMessage::Tool { content, .. } if content == OMITTED_TOOL_RESULT)
        );
        assert!(matches!(&context[5], ChatMessage::Tool { content, .. } if content == "current"));

        let (context, start) = fit_context(&messages, 3, Some(8));
        assert_eq!(start, 0);
        assert_eq!(context.len(), 3);
        assert!(matches!(&context[0], ChatMessage::User { content, .. } if content == "second"));
    }

    #[test]
    fn earlier_images_are_omitted_before_earlier_turns_are_dropped() {
        let image = crate::images::inspect(&crate::images::tests::png(2048, 2048)).expect("image");
        let with_image = |text: &str| ChatMessage::User {
            content: text.to_owned(),
            automatic: false,
            images: vec![image.clone()],
        };
        let messages = vec![
            with_image("first"),
            ChatMessage::Assistant {
                content: "seen".to_owned(),
                reasoning: None,
                tool_calls: Vec::new(),
            },
            with_image("second"),
        ];
        // Each image is estimated at 5,350 tokens; both fit in 64,000.
        let (context, _) = fit_context(&messages, 2, None);
        assert_eq!(context, messages);
        let (context, start) = fit_context(&messages, 2, Some(12_000));
        assert_eq!(start, 2);
        assert!(
            matches!(&context[0], ChatMessage::User { content, images, .. }
            if images.is_empty() && content.ends_with("[1 earlier image(s) omitted to save context]"))
        );
        assert!(matches!(&context[2], ChatMessage::User { images, .. } if images.len() == 1));
    }

    struct Tools;

    impl ToolExecutor for Tools {
        fn execute<'a>(
            &'a self,
            call: &'a ToolCall,
        ) -> Pin<Box<dyn Future<Output = ToolOutput> + Send + 'a>> {
            Box::pin(async move {
                ToolOutput {
                    content: format!("{{\"ran\":\"{}\"}}", call.name),
                    is_error: false,
                }
            })
        }
    }

    /// Serves the given SSE bodies to consecutive requests and records the
    /// request bodies.
    fn serve(bodies: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("address").port();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for body in bodies {
                let (stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("line");
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().expect("length");
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut request = vec![0; length];
                reader.read_exact(&mut request).expect("body");
                requests.push(String::from_utf8(request).expect("utf-8"));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .expect("write");
            }
            requests
        });
        (format!("http://127.0.0.1:{port}/v1"), handle)
    }

    #[test]
    fn a_turn_runs_tools_until_the_model_answers() {
        let (url, server) = serve(vec![
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"look\",\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"list_sheets\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n".to_owned(),
            "data: {\"choices\":[{\"delta\":{\"content\":\"Готово\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n".to_owned(),
        ]);
        let endpoint = ProviderEndpoint {
            base_url: BaseUrl::parse(&url).expect("url"),
            api_key: ApiKey::new("sk").expect("key"),
            session_header: Some("x-opencode-session".to_owned()),
            headers: Vec::new(),
            protocol: crate::provider::Protocol::ChatCompletions,
        };
        let client = OpenAiCompatibleClient::new().expect("client");
        let config = TurnConfig {
            model: "glm-5.3",
            effort: None,
            system: "You are Angelica.",
            tools: &[],
            context_tokens: None,
            session: "conversation-1",
            max_rounds: MAX_ROUNDS_PER_TURN,
            images: None,
        };
        let mut messages = vec![user("Сколько листов?")];
        let events = Mutex::new(Vec::new());
        let mut persisted = 0;
        let summary = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(run_turn(
                &client,
                &endpoint,
                &config,
                &mut messages,
                &Tools,
                &mut |event| events.lock().expect("lock").push(event),
                &mut |_| persisted += 1,
            ))
            .expect("turn");

        assert_eq!(summary.outcome, TurnOutcome::Completed);
        assert_eq!(summary.usage.prompt_tokens, 50);
        assert_eq!(messages.len(), 4);
        assert!(
            matches!(&messages[2], ChatMessage::Tool { content, .. } if content.contains("list_sheets"))
        );
        assert!(
            matches!(&messages[3], ChatMessage::Assistant { content, .. } if content == "Готово")
        );
        assert_eq!(persisted, 3);
        let events = events.into_inner().expect("events");
        assert!(events.contains(&AgentEvent::ToolStarted {
            id: "c1".to_owned(),
            name: "list_sheets".to_owned(),
            arguments: "{}".to_owned(),
        }));

        let requests = server.join().expect("server");
        let second: serde_json::Value = serde_json::from_str(&requests[1]).expect("json");
        let sent = second["messages"].as_array().expect("messages");
        assert_eq!(sent[2]["reasoning_content"], "look");
        assert_eq!(sent[3]["role"], "tool");
    }
}
