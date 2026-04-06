use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;

// ── Tool types ────────────────────────────────────────────────────────────────

/// A tool that can be offered to an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    /// JSON Schema describing the tool's parameters.
    pub parameters: JsonValue,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this call (used to correlate with results).
    pub id: String,
    pub name: String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}

/// The result of executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Must match the `id` from the originating `ToolCall`.
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

// ── Streaming types ───────────────────────────────────────────────────────────

/// Events emitted by a streaming LLM response.
#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    /// Incremental text content.
    TextDelta(String),
    /// A complete tool call has been parsed from the stream.
    ToolUse(ToolCall),
    /// Token usage (may arrive mid-stream or at the end).
    Usage(LlmUsage),
    /// The stream has ended; contains the finish reason.
    Done(String),
    /// An error occurred during streaming.
    Error(String),
}

// ── Message ───────────────────────────────────────────────────────────────────

/// Tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool call returned by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// A tool result to feed back to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

/// A single message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
    /// Tool calls present in an assistant message.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool results present in a user message.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_results: Option<Vec<ToolResult>>,
}

impl LlmMessage {
    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            ..Default::default()
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            ..Default::default()
        }
    }

    /// Create an assistant message that also carries tool calls.
    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls: Some(tool_calls),
            tool_results: None,
        }
    }

    /// Create a user message that carries tool results (no text content).
    pub fn tool_results(results: Vec<ToolResult>) -> Self {
        Self {
            role: "user".into(),
            content: String::new(),
            tool_calls: None,
            tool_results: Some(results),
        }
    }
}

/// Request sent to an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub system_prompt: Option<String>,
    pub messages: Vec<LlmMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    /// Tools available to the LLM.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tools: Option<Vec<LlmTool>>,
}

/// Token usage reported by the LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// Response from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    /// Text content (may be empty when the response is tool calls only).
    pub content: String,
    /// Tool calls requested by the model.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    pub usage: LlmUsage,
    /// "stop", "end_turn", or "tool_use".
    pub finish_reason: String,
    /// Tool calls requested by the model (when `finish_reason` is "tool_calls").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// Abstraction over any LLM provider.
///
/// Implement this trait to plug in OpenAI, Claude, Doubao, Ollama, or any
/// other provider. The `workflow-engine-llm-openai` crate ships a ready-made
/// implementation for OpenAI-compatible APIs.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;
}

/// Extension trait for LLM providers that support token-by-token streaming.
///
/// Streaming providers send events through an `mpsc::Sender` rather than
/// returning a completed response, enabling parallel tool execution as
/// tool calls arrive from the stream.
#[async_trait]
pub trait StreamingLlmProvider: LlmProvider {
    /// Stream a completion. Events are sent through `tx` as they arrive.
    ///
    /// Completes (returns `Ok`) after sending `LlmStreamEvent::Done` or
    /// `LlmStreamEvent::Error`. The caller owns the receiver end.
    async fn stream(&self, request: LlmRequest, tx: mpsc::Sender<LlmStreamEvent>) -> Result<()>;
}
// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── LlmMessage constructors ──────────────────────────────────────────────

    #[test]
    fn llm_message_user_constructor() {
        let m = LlmMessage::user("hello");
        assert_eq!(m.role, "user");
        assert_eq!(m.content, "hello");
        assert!(m.tool_calls.is_none());
        assert!(m.tool_results.is_none());
    }

    #[test]
    fn llm_message_assistant_constructor() {
        let m = LlmMessage::assistant("hi there");
        assert_eq!(m.role, "assistant");
        assert_eq!(m.content, "hi there");
        assert!(m.tool_calls.is_none());
        assert!(m.tool_results.is_none());
    }

    #[test]
    fn llm_message_assistant_with_tool_calls_constructor() {
        let tc = ToolCall {
            id: "call_1".into(),
            name: "get_weather".into(),
            input: json!({"city": "London"}),
        };
        let m = LlmMessage::assistant_with_tool_calls("", vec![tc.clone()]);
        assert_eq!(m.role, "assistant");
        let calls = m.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "get_weather");
    }

    #[test]
    fn llm_message_tool_results_constructor() {
        let tr = ToolResult {
            tool_use_id: "call_1".into(),
            content: "sunny".into(),
            is_error: false,
        };
        let m = LlmMessage::tool_results(vec![tr]);
        assert_eq!(m.role, "user");
        assert!(m.content.is_empty());
        let results = m.tool_results.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_use_id, "call_1");
        assert!(!results[0].is_error);
    }

    // ── LlmRequest with tools serializes correctly ───────────────────────────

    #[test]
    fn llm_request_with_tools_serializes() {
        let tool = LlmTool {
            name: "get_weather".into(),
            description: "Get weather for a city".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"]
            }),
        };
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system_prompt: None,
            messages: vec![LlmMessage::user("What is the weather in London?")],
            temperature: None,
            max_tokens: None,
            tools: Some(vec![tool]),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["tools"][0]["name"], "get_weather");
        assert_eq!(v["tools"][0]["description"], "Get weather for a city");
        assert_eq!(v["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn llm_request_without_tools_omits_tools_field() {
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system_prompt: None,
            messages: vec![LlmMessage::user("hello")],
            temperature: None,
            max_tokens: None,
            tools: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert!(v.get("tools").is_none());
    }

    // ── LlmTool JSON schema serializes correctly ─────────────────────────────

    #[test]
    fn llm_tool_serializes() {
        let tool = LlmTool {
            name: "search".into(),
            description: "Search the web".into(),
            input_schema: json!({"type": "object"}),
        };
        let v = serde_json::to_value(&tool).unwrap();
        assert_eq!(v["name"], "search");
        assert_eq!(v["description"], "Search the web");
        assert_eq!(v["input_schema"]["type"], "object");
    }

    // ── ToolCall and ToolResult round-trip ───────────────────────────────────

    #[test]
    fn tool_call_roundtrip() {
        let tc = ToolCall {
            id: "c1".into(),
            name: "fn_name".into(),
            input: json!({"x": 1, "y": "hello"}),
        };
        let s = serde_json::to_string(&tc).unwrap();
        let back: ToolCall = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, "c1");
        assert_eq!(back.name, "fn_name");
        assert_eq!(back.input["x"], 1);
    }

    #[test]
    fn tool_result_roundtrip() {
        let tr = ToolResult {
            tool_use_id: "c1".into(),
            content: "42 degrees".into(),
            is_error: true,
        };
        let s = serde_json::to_string(&tr).unwrap();
        let back: ToolResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back.tool_use_id, "c1");
        assert_eq!(back.content, "42 degrees");
        assert!(back.is_error);
    }

    // ── Backward compat: old struct literal syntax still compiles ────────────

    #[test]
    fn backward_compat_struct_literal() {
        // Old code that creates LlmMessage without new fields must still compile.
        // The fields have `#[serde(default)]` and the struct derives Default.
        let m = LlmMessage {
            role: "user".into(),
            content: "test".into(),
            ..Default::default()
        };
        assert_eq!(m.role, "user");
        assert!(m.tool_calls.is_none());
    }

    #[test]
    fn backward_compat_llm_response_without_tool_calls() {
        let r = LlmResponse {
            content: "hello".into(),
            tool_calls: None,
            usage: LlmUsage {
                prompt_tokens: 5,
                completion_tokens: 3,
            },
            finish_reason: "stop".into(),
        };
        assert_eq!(r.finish_reason, "stop");
        assert!(r.tool_calls.is_none());
    }
}
