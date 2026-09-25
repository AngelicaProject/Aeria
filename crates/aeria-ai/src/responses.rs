//! The Codex backend's Responses API: request bodies and streamed events.

use serde_json::{Value, json};

use crate::chat::{
    AssistantResponse, ChatMessage, ChatRequest, MAX_TOOL_CALLS_PER_RESPONSE, StreamDelta,
    StreamError, ToolCall, Usage,
};

/// Builds a streaming Responses request from Aeria's conversation.
///
/// The system message becomes `instructions`; messages become typed input
/// items. Nothing is stored server-side (`store: false`), so reasoning is
/// not replayed between requests.
#[must_use]
pub fn request_body(request: &ChatRequest<'_>, session: &str) -> Value {
    let mut input = Vec::with_capacity(request.messages.len());
    for message in request.messages {
        match message {
            ChatMessage::User { content, .. } => input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": content }],
            })),
            ChatMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if !content.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": content }],
                    }));
                }
                for call in tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                    }));
                }
            }
            ChatMessage::Tool {
                tool_call_id,
                content,
                ..
            } => input.push(json!({
                "type": "function_call_output",
                "call_id": tool_call_id,
                "output": content,
            })),
        }
    }
    let mut reasoning = json!({ "summary": "auto" });
    if let Some(effort) = request.effort {
        reasoning["effort"] = Value::from(effort.as_str());
    }
    let mut body = json!({
        "model": request.model,
        "instructions": request.system,
        "input": input,
        "store": false,
        "stream": true,
        "reasoning": reasoning,
        "include": [],
        "prompt_cache_key": session,
    });
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                    "strict": false,
                })
            })
            .collect();
        body["tool_choice"] = Value::from("auto");
        body["parallel_tool_calls"] = Value::from(true);
    }
    body
}

/// Accumulates Responses server-sent events into an [`AssistantResponse`].
#[derive(Default)]
pub struct ResponsesAccumulator {
    buffer: Vec<u8>,
    content: String,
    reasoning: String,
    calls: Vec<ToolCall>,
    usage: Option<Usage>,
    model: Option<String>,
    status: Option<String>,
    completed: bool,
}

impl ResponsesAccumulator {
    /// Feeds raw response bytes; complete `data:` lines are parsed.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed events or a failed response.
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

    /// Returns whether the terminal event arrived.
    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.completed
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
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let event: Value = serde_json::from_str(data)
            .map_err(|error| StreamError(format!("invalid stream event: {error}")))?;
        let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or_default();
        match text("type") {
            "response.output_text.delta" => {
                let delta = text("delta");
                if !delta.is_empty() {
                    self.content.push_str(delta);
                    on_delta(StreamDelta::Text(delta.to_owned()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = text("delta");
                if !delta.is_empty() {
                    self.reasoning.push_str(delta);
                    on_delta(StreamDelta::Reasoning(delta.to_owned()));
                }
            }
            "response.reasoning_summary_part.done" => {
                if !self.reasoning.is_empty() && !self.reasoning.ends_with("\n\n") {
                    self.reasoning.push_str("\n\n");
                    on_delta(StreamDelta::Reasoning("\n\n".to_owned()));
                }
            }
            "response.output_item.done" => {
                let item = event.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    if self.calls.len() >= MAX_TOOL_CALLS_PER_RESPONSE {
                        return Err(StreamError(format!(
                            "the model requested more than {MAX_TOOL_CALLS_PER_RESPONSE} tool calls"
                        )));
                    }
                    let field = |key: &str| {
                        item.get(key)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    let arguments = field("arguments");
                    self.calls.push(ToolCall {
                        id: Some(field("call_id"))
                            .filter(|id| !id.is_empty())
                            .unwrap_or_else(|| format!("call_{}", self.calls.len())),
                        name: field("name"),
                        arguments: if arguments.trim().is_empty() {
                            "{}".to_owned()
                        } else {
                            arguments
                        },
                    });
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = event.get("response").unwrap_or(&Value::Null);
                self.status = response
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.model = response
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(usage) = response.get("usage") {
                    self.usage = Some(Usage {
                        prompt_tokens: usage
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        completion_tokens: usage
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    });
                }
                self.completed = true;
            }
            "response.failed" | "error" => {
                let error = event
                    .pointer("/response/error")
                    .or_else(|| event.get("error"))
                    .unwrap_or(&event);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| error.to_string(), str::to_owned);
                return Err(StreamError(message));
            }
            _ => {}
        }
        Ok(())
    }

    /// Finishes the stream.
    ///
    /// # Errors
    ///
    /// Returns an error when the stream ended before its terminal event.
    pub fn finish(
        mut self,
        on_delta: &mut dyn FnMut(StreamDelta),
    ) -> Result<(AssistantResponse, Option<String>), StreamError> {
        let rest = String::from_utf8(std::mem::take(&mut self.buffer))
            .map_err(|_| StreamError("stream line is not UTF-8".to_owned()))?;
        self.line(rest.trim_end_matches('\r'), on_delta)?;
        if !self.completed {
            return Err(StreamError(
                "the response ended before it completed".to_owned(),
            ));
        }
        let finish_reason = match self.status.as_deref() {
            Some("incomplete") => Some("length".to_owned()),
            _ if !self.calls.is_empty() => Some("tool_calls".to_owned()),
            _ => Some("stop".to_owned()),
        };
        Ok((
            AssistantResponse {
                content: self.content,
                reasoning: (!self.reasoning.trim().is_empty())
                    .then(|| self.reasoning.trim().to_owned()),
                tool_calls: self
                    .calls
                    .into_iter()
                    .filter(|call| !call.name.is_empty())
                    .collect(),
                finish_reason,
                usage: self.usage,
            },
            self.model,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ToolDefinition;
    use crate::provider::ReasoningEffort;

    #[test]
    fn body_maps_messages_tools_and_effort_to_responses_items() {
        let messages = vec![
            ChatMessage::User {
                content: "q".to_owned(),
                automatic: false,
            },
            ChatMessage::Assistant {
                content: "looking".to_owned(),
                reasoning: Some("private".to_owned()),
                tool_calls: vec![ToolCall {
                    id: "c1".to_owned(),
                    name: "get_unit".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            },
            ChatMessage::Tool {
                tool_call_id: "c1".to_owned(),
                name: "get_unit".to_owned(),
                content: "{\"row\":1}".to_owned(),
            },
        ];
        let tools = [ToolDefinition {
            name: "get_unit",
            description: "Read one string.",
            parameters: json!({ "type": "object", "properties": {} }),
        }];
        let body = request_body(
            &ChatRequest {
                model: "gpt-5.5",
                effort: Some(ReasoningEffort::XHigh),
                system: "You are Angelica.",
                messages: &messages,
                tools: &tools,
                turn_start: 0,
            },
            "conversation-1",
        );
        assert_eq!(body["instructions"], "You are Angelica.");
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"]["effort"], "xhigh");
        assert_eq!(body["prompt_cache_key"], "conversation-1");
        let input = body["input"].as_array().expect("input");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "c1");
        assert!(!body.to_string().contains("private"));
        assert_eq!(body["tools"][0]["name"], "get_unit");
    }

    fn run(
        events: &[Value],
    ) -> Result<(AssistantResponse, Option<String>, Vec<StreamDelta>), StreamError> {
        let mut accumulator = ResponsesAccumulator::default();
        let mut deltas = Vec::new();
        for event in events {
            accumulator.push(
                format!("event: x\ndata: {event}\n\n").as_bytes(),
                &mut |delta| deltas.push(delta),
            )?;
        }
        let (response, model) = accumulator.finish(&mut |delta| deltas.push(delta))?;
        Ok((response, model, deltas))
    }

    #[test]
    fn events_become_text_reasoning_tool_calls_and_usage() {
        let (response, model, deltas) = run(&[
            json!({ "type": "response.reasoning_summary_text.delta", "delta": "plan" }),
            json!({ "type": "response.reasoning_summary_part.done" }),
            json!({ "type": "response.output_text.delta", "delta": "Смотрю" }),
            json!({ "type": "response.output_item.done", "item": { "type": "function_call", "call_id": "c9", "name": "list_sheets", "arguments": "" } }),
            json!({ "type": "response.completed", "response": { "status": "completed", "model": "gpt-5.5", "usage": { "input_tokens": 40, "output_tokens": 7 } } }),
        ])
        .expect("stream");
        assert_eq!(response.content, "Смотрю");
        assert_eq!(response.reasoning.as_deref(), Some("plan"));
        assert_eq!(response.tool_calls[0].id, "c9");
        assert_eq!(response.tool_calls[0].arguments, "{}");
        assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            response.usage,
            Some(Usage {
                prompt_tokens: 40,
                completion_tokens: 7
            })
        );
        assert_eq!(model.as_deref(), Some("gpt-5.5"));
        assert_eq!(deltas[0], StreamDelta::Reasoning("plan".to_owned()));
    }

    #[test]
    fn failed_or_unfinished_streams_are_errors() {
        let failed = run(&[
            json!({ "type": "response.failed", "response": { "error": { "message": "usage limit reached" } } }),
        ]);
        assert_eq!(
            failed.err(),
            Some(StreamError("usage limit reached".to_owned()))
        );
        let unfinished = run(&[json!({ "type": "response.output_text.delta", "delta": "hi" })]);
        assert!(unfinished.is_err());
    }
}
