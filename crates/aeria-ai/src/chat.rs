//! Chat Completions messages, tool definitions, and streaming responses.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::provider::ReasoningEffort;

/// One tool call requested by the model.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The raw JSON arguments text, exactly as the model produced it.
    pub arguments: String,
}

/// One conversation message in Aeria's own representation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "role"
)]
pub enum ChatMessage {
    User {
        content: String,
        /// Written by Aeria, not the user, for example a job update.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        automatic: bool,
    },
    Assistant {
        content: String,
        /// Reasoning text the provider streamed, when it exposes it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        name: String,
        content: String,
    },
}

/// A tool offered to the model.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the arguments object.
    pub parameters: Value,
}

/// A complete streaming request.
#[derive(Clone, Debug)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub effort: Option<ReasoningEffort>,
    pub system: &'a str,
    pub messages: &'a [ChatMessage],
    pub tools: &'a [ToolDefinition],
    /// Messages from this index on belong to the current turn; their
    /// reasoning is sent back, which some providers require while tools run.
    pub turn_start: usize,
}

impl ChatRequest<'_> {
    /// Builds the OpenAI-compatible request body.
    #[must_use]
    pub fn body(&self) -> Value {
        let mut messages = Vec::with_capacity(self.messages.len() + 1);
        messages.push(json!({ "role": "system", "content": self.system }));
        for (index, message) in self.messages.iter().enumerate() {
            messages.push(match message {
                ChatMessage::User { content, .. } => json!({ "role": "user", "content": content }),
                ChatMessage::Assistant {
                    content,
                    reasoning,
                    tool_calls,
                } => {
                    let mut value = Map::new();
                    value.insert("role".to_owned(), Value::from("assistant"));
                    value.insert("content".to_owned(), Value::from(content.as_str()));
                    if !tool_calls.is_empty() {
                        value.insert(
                            "tool_calls".to_owned(),
                            tool_calls
                                .iter()
                                .map(|call| {
                                    json!({
                                        "id": call.id,
                                        "type": "function",
                                        "function": { "name": call.name, "arguments": call.arguments },
                                    })
                                })
                                .collect(),
                        );
                    }
                    if index >= self.turn_start
                        && let Some(reasoning) = reasoning
                    {
                        value.insert(
                            "reasoning_content".to_owned(),
                            Value::from(reasoning.as_str()),
                        );
                    }
                    Value::Object(value)
                }
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                    ..
                } => json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content }),
            });
        }
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if !self.tools.is_empty() {
            body["tools"] = self
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        },
                    })
                })
                .collect();
        }
        if let Some(effort) = self.effort {
            body["reasoning_effort"] = Value::from(effort.as_str());
        }
        body
    }
}

/// Token usage reported by the provider.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Self) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
    }
}

/// Incremental output while a response streams.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamDelta {
    Text(String),
    Reasoning(String),
    /// A piece of tool-call arguments. The arguments are only valid JSON
    /// once the response completes; the pieces serve live progress.
    ToolArguments(String),
}

/// The complete assistant response assembled from a stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AssistantResponse {
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Accumulates server-sent-event chunks into an [`AssistantResponse`].
#[derive(Default)]
pub struct StreamAccumulator {
    buffer: Vec<u8>,
    content: String,
    reasoning: String,
    calls: Vec<PartialCall>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    done: bool,
}

/// A malformed stream chunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamError(pub String);

/// Most tool calls accepted in one response.
pub const MAX_TOOL_CALLS_PER_RESPONSE: usize = 32;

impl StreamAccumulator {
    /// Feeds raw response text; complete `data:` lines are parsed and their
    /// deltas passed to `on_delta`.
    ///
    /// # Errors
    ///
    /// Returns an error for a chunk that is not valid JSON or reports an
    /// error object.
    pub fn push(
        &mut self,
        bytes: &[u8],
        on_delta: &mut dyn FnMut(StreamDelta),
    ) -> Result<(), StreamError> {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = std::str::from_utf8(&line)
                .map_err(|_| StreamError("stream line is not UTF-8".to_owned()))?;
            self.line(line.trim_end_matches(['\n', '\r']), on_delta)?;
        }
        Ok(())
    }

    fn line(
        &mut self,
        line: &str,
        on_delta: &mut dyn FnMut(StreamDelta),
    ) -> Result<(), StreamError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(());
        };
        let data = data.trim();
        if data.is_empty() {
            return Ok(());
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|error| StreamError(format!("invalid stream chunk: {error}")))?;
        if let Some(error) = chunk.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| error.to_string(), str::to_owned);
            return Err(StreamError(message));
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(Usage {
                prompt_tokens: usage
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                completion_tokens: usage
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            });
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return Ok(());
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_owned());
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(());
        };
        for key in ["reasoning_content", "reasoning"] {
            if let Some(text) = delta.get(key).and_then(Value::as_str)
                && !text.is_empty()
            {
                self.reasoning.push_str(text);
                on_delta(StreamDelta::Reasoning(text.to_owned()));
                break;
            }
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            self.content.push_str(text);
            on_delta(StreamDelta::Text(text.to_owned()));
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|index| usize::try_from(index).ok())
                    .unwrap_or(self.calls.len().saturating_sub(1));
                if index >= MAX_TOOL_CALLS_PER_RESPONSE {
                    return Err(StreamError(format!(
                        "the model requested more than {MAX_TOOL_CALLS_PER_RESPONSE} tool calls"
                    )));
                }
                while self.calls.len() <= index {
                    self.calls.push(PartialCall::default());
                }
                let partial = &mut self.calls[index];
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    partial.id.push_str(id);
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        partial.name.push_str(name);
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str)
                        && !arguments.is_empty()
                    {
                        partial.arguments.push_str(arguments);
                        on_delta(StreamDelta::ToolArguments(arguments.to_owned()));
                    }
                }
            }
        }
        Ok(())
    }

    /// Returns whether the stream sent its `[DONE]` marker.
    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.done
    }

    /// Finishes the stream, flushing a final line without a newline.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed final chunk.
    pub fn finish(
        mut self,
        on_delta: &mut dyn FnMut(StreamDelta),
    ) -> Result<AssistantResponse, StreamError> {
        let rest = String::from_utf8(std::mem::take(&mut self.buffer))
            .map_err(|_| StreamError("stream line is not UTF-8".to_owned()))?;
        self.line(rest.trim_end_matches('\r'), on_delta)?;
        let tool_calls = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, call)| !call.name.is_empty())
            .map(|(index, call)| ToolCall {
                id: if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                },
                name: call.name,
                arguments: if call.arguments.trim().is_empty() {
                    "{}".to_owned()
                } else {
                    call.arguments
                },
            })
            .collect();
        Ok(AssistantResponse {
            content: self.content,
            reasoning: (!self.reasoning.is_empty()).then_some(self.reasoning),
            tool_calls,
            finish_reason: self.finish_reason,
            usage: self.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(chunks: &[&str]) -> (AssistantResponse, Vec<StreamDelta>) {
        let mut accumulator = StreamAccumulator::default();
        let mut deltas = Vec::new();
        for chunk in chunks {
            accumulator
                .push(chunk.as_bytes(), &mut |delta| deltas.push(delta))
                .expect("chunk");
        }
        let response = accumulator
            .finish(&mut |delta| deltas.push(delta))
            .expect("finish");
        (response, deltas)
    }

    #[test]
    fn text_reasoning_and_usage_are_accumulated_across_split_chunks() {
        let (response, deltas) = collect(&[
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n\ndata: {\"choi",
            "ces\":[{\"delta\":{\"content\":\"При\"}}]}\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"вет\"},\"finish_reason\":\"stop\"}]}\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n",
            "data: [DONE]\n",
        ]);
        assert_eq!(response.content, "Привет");
        assert_eq!(response.reasoning.as_deref(), Some("think"));
        assert_eq!(response.finish_reason.as_deref(), Some("stop"));
        assert_eq!(
            response.usage,
            Some(Usage {
                prompt_tokens: 12,
                completion_tokens: 3
            })
        );
        assert_eq!(
            deltas,
            vec![
                StreamDelta::Reasoning("think".to_owned()),
                StreamDelta::Text("При".to_owned()),
                StreamDelta::Text("вет".to_owned()),
            ]
        );
    }

    #[test]
    fn tool_call_fragments_are_joined_by_index() {
        let (response, deltas) = collect(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"get_unit\",\"arguments\":\"{\\\"sheet\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"c2\",\"function\":{\"name\":\"list_sheets\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\":\\\"Item\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}",
        ]);
        assert_eq!(
            response.tool_calls,
            vec![
                ToolCall {
                    id: "c1".to_owned(),
                    name: "get_unit".to_owned(),
                    arguments: "{\"sheet\":\"Item\"}".to_owned(),
                },
                ToolCall {
                    id: "c2".to_owned(),
                    name: "list_sheets".to_owned(),
                    arguments: "{}".to_owned(),
                },
            ]
        );
        assert_eq!(
            deltas,
            vec![
                StreamDelta::ToolArguments("{\"sheet".to_owned()),
                StreamDelta::ToolArguments("\":\"Item\"}".to_owned())
            ]
        );
    }

    #[test]
    fn stream_error_objects_and_malformed_chunks_fail() {
        let mut accumulator = StreamAccumulator::default();
        let error = accumulator
            .push(
                b"data: {\"error\":{\"message\":\"overloaded\"}}\n",
                &mut |_| {},
            )
            .expect_err("error chunk");
        assert_eq!(error, StreamError("overloaded".to_owned()));
        let mut accumulator = StreamAccumulator::default();
        assert!(accumulator.push(b"data: {nope\n", &mut |_| {}).is_err());
    }

    #[test]
    fn multibyte_characters_split_between_chunks_are_decoded() {
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"ё\"}}]}\n".as_bytes();
        let split = line
            .iter()
            .position(|byte| *byte >= 0x80)
            .expect("multibyte")
            + 1;
        let mut accumulator = StreamAccumulator::default();
        accumulator
            .push(&line[..split], &mut |_| {})
            .expect("first half");
        accumulator
            .push(&line[split..], &mut |_| {})
            .expect("second half");
        assert_eq!(
            accumulator.finish(&mut |_| {}).expect("finish").content,
            "ё"
        );
    }

    #[test]
    fn request_body_sends_reasoning_only_for_the_current_turn() {
        let messages = vec![
            ChatMessage::User {
                content: "q1".to_owned(),
                automatic: false,
            },
            ChatMessage::Assistant {
                content: "a1".to_owned(),
                reasoning: Some("old".to_owned()),
                tool_calls: Vec::new(),
            },
            ChatMessage::User {
                content: "q2".to_owned(),
                automatic: false,
            },
            ChatMessage::Assistant {
                content: String::new(),
                reasoning: Some("new".to_owned()),
                tool_calls: vec![ToolCall {
                    id: "c1".to_owned(),
                    name: "list_sheets".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            },
            ChatMessage::Tool {
                tool_call_id: "c1".to_owned(),
                name: "list_sheets".to_owned(),
                content: "[]".to_owned(),
            },
        ];
        let tools = [ToolDefinition {
            name: "list_sheets",
            description: "List sheets.",
            parameters: json!({ "type": "object", "properties": {} }),
        }];
        let body = ChatRequest {
            model: "glm-5.3",
            effort: Some(ReasoningEffort::High),
            system: "You are Angelica.",
            messages: &messages,
            tools: &tools,
            turn_start: 2,
        }
        .body();
        let sent = body["messages"].as_array().expect("messages");
        assert_eq!(sent[0]["role"], "system");
        assert!(sent[2].get("reasoning_content").is_none());
        assert_eq!(sent[4]["reasoning_content"], "new");
        assert_eq!(sent[4]["tool_calls"][0]["function"]["name"], "list_sheets");
        assert_eq!(sent[5]["tool_call_id"], "c1");
        assert_eq!(body["tools"][0]["function"]["name"], "list_sheets");
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["stream"], true);
    }
}
