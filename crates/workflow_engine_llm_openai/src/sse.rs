/// Server-Sent Events (SSE) parser for OpenAI streaming completions.
use serde::Deserialize;
use serde_json::Value as JsonValue;

use workflow_engine::llm::{LlmStreamEvent, LlmUsage, ToolCall};

/// A parsed SSE chunk from the streaming response.
#[derive(Debug)]
pub enum SseChunk {
    TextDelta(String),
    FinishReason(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallArgsDelta {
        index: usize,
        args_delta: String,
    },
    Usage {
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    Done,
}

/// Intermediate deserialization types for the OpenAI streaming format.
#[derive(Deserialize, Debug)]
struct StreamChoice {
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct StreamDelta {
    content: Option<String>,
    #[allow(dead_code)]
    role: Option<String>,
    tool_calls: Option<Vec<StreamToolCallChunk>>,
}

#[derive(Deserialize, Debug)]
struct StreamToolCallChunk {
    index: Option<u32>,
    id: Option<String>,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    call_type: Option<String>,
    function: Option<StreamFunctionChunk>,
}

#[derive(Deserialize, Debug)]
struct StreamFunctionChunk {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct StreamUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Deserialize, Debug)]
struct StreamResponse {
    choices: Option<Vec<StreamChoice>>,
    usage: Option<StreamUsage>,
}

/// Parse a single SSE `data: ...` line into an `SseChunk`.
///
/// Returns `None` for empty lines, comment lines, and non-data lines.
pub fn parse_sse_chunk(line: &str) -> Option<SseChunk> {
    if line.is_empty() || line.starts_with(':') {
        return None;
    }
    let data = line.strip_prefix("data: ")?;

    if data == "[DONE]" {
        return Some(SseChunk::Done);
    }

    let v: JsonValue = serde_json::from_str(data).ok()?;
    let resp: StreamResponse = serde_json::from_value(v).ok()?;

    // Usage-only chunks (no choices)
    if resp.choices.is_none() || resp.choices.as_ref().map(|c| c.is_empty()).unwrap_or(true) {
        if let Some(usage) = resp.usage {
            return Some(SseChunk::Usage {
                prompt_tokens: usage.prompt_tokens.unwrap_or(0),
                completion_tokens: usage.completion_tokens.unwrap_or(0),
            });
        }
        return None;
    }

    let choice = resp.choices?.into_iter().next()?;

    // Tool call chunks
    if let Some(tool_calls) = choice.delta.tool_calls {
        if let Some(tc) = tool_calls.into_iter().next() {
            let index = tc.index.unwrap_or(0) as usize;
            if let Some(func) = tc.function {
                // Start chunk has name + empty args
                if let Some(name) = func.name {
                    if !name.is_empty() {
                        return Some(SseChunk::ToolCallStart {
                            id: tc.id.unwrap_or_default(),
                            name,
                        });
                    }
                }
                // Args delta chunk
                if let Some(args) = func.arguments {
                    return Some(SseChunk::ToolCallArgsDelta {
                        index,
                        args_delta: args,
                    });
                }
            }
        }
    }

    // Finish reason
    if let Some(reason) = choice.finish_reason {
        if !reason.is_empty() {
            // Treat finish_reason=stop/tool_calls as Done
            return Some(if reason == "stop" || reason == "tool_calls" {
                SseChunk::Done
            } else {
                SseChunk::FinishReason(reason)
            });
        }
    }

    // Text delta (including empty strings for role-only chunks)
    let content = choice.delta.content.unwrap_or_default();
    Some(SseChunk::TextDelta(content))
}

/// Collect `LlmStreamEvent`s by processing a full SSE body string.
///
/// Used in tests and when buffered streaming is acceptable.
/// Tool calls are assembled from start + args-delta chunks before emitting `ToolUse`.
pub fn collect_events_from_sse(sse_body: &str) -> Vec<LlmStreamEvent> {
    let mut events = Vec::new();
    // Accumulate tool call arguments: index → (id, name, accumulated_args)
    let mut tool_call_buf: std::collections::HashMap<usize, (String, String, String)> =
        std::collections::HashMap::new();
    let mut usage_record: Option<(u32, u32)> = None;

    for line in sse_body.lines() {
        let line = line.trim();
        match parse_sse_chunk(line) {
            None => {}
            Some(SseChunk::TextDelta(s)) if !s.is_empty() => {
                events.push(LlmStreamEvent::TextDelta(s));
            }
            Some(SseChunk::TextDelta(_)) => {} // skip empty/role-only
            Some(SseChunk::ToolCallStart { id, name }) => {
                // Start accumulating for index 0 (start chunks always index=0 effectively)
                tool_call_buf.entry(0).or_insert((id, name, String::new()));
            }
            Some(SseChunk::ToolCallArgsDelta { index, args_delta }) => {
                if let Some(entry) = tool_call_buf.get_mut(&index) {
                    entry.2.push_str(&args_delta);
                }
            }
            Some(SseChunk::Usage {
                prompt_tokens,
                completion_tokens,
            }) => {
                usage_record = Some((prompt_tokens, completion_tokens));
            }
            Some(SseChunk::FinishReason(_)) => {}
            Some(SseChunk::Done) => {
                // Emit any accumulated tool calls before Done
                let mut tool_indices: Vec<usize> = tool_call_buf.keys().cloned().collect();
                tool_indices.sort();
                for idx in tool_indices {
                    if let Some((id, name, args)) = tool_call_buf.remove(&idx) {
                        events.push(LlmStreamEvent::ToolUse(ToolCall {
                            id,
                            name,
                            arguments: args,
                        }));
                    }
                }
                if let Some((prompt_tokens, completion_tokens)) = usage_record.take() {
                    events.push(LlmStreamEvent::Usage(LlmUsage {
                        prompt_tokens,
                        completion_tokens,
                    }));
                }
                events.push(LlmStreamEvent::Done("stop".into()));
                break;
            }
        }
    }

    events
}
