pub mod sse;
pub mod streaming;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, info};

use workflow_engine::llm::{LlmProvider, LlmRequest, LlmResponse, LlmTool, LlmUsage, ToolCall};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const REQUEST_TIMEOUT_SECS: u64 = 120;

// ── Wire types (internal) ─────────────────────────────────────────────────────

/// Wrapper for tool function definition sent to OpenAI.
#[derive(Debug, Serialize)]
struct WireToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

/// A tool entry in the OpenAI request.
#[derive(Debug, Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    kind: String,
    function: WireToolFunction,
}

impl WireTool {
    fn from_llm_tool(tool: &LlmTool) -> Self {
        WireTool {
            kind: "function".into(),
            function: WireToolFunction {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            },
        }
    }
}

/// Tool call function from the response.
#[derive(Debug, Clone, Deserialize)]
struct WireToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

/// A tool_call entry in the OpenAI response choice message.
#[derive(Debug, Clone, Deserialize)]
struct WireToolCall {
    id: Option<String>,
    function: Option<WireToolCallFunction>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum WireMessage {
    /// Regular user / system / assistant (text-only) message.
    Plain { role: String, content: String },
    /// Assistant message that carries tool calls.
    AssistantWithTools {
        role: String,
        content: String,
        tool_calls: Vec<WireAssistantToolCall>,
    },
    /// Tool result message.
    Tool {
        role: String,
        tool_call_id: String,
        content: String,
    },
}

/// Tool call entry serialised inside an assistant message.
#[derive(Debug, Serialize)]
struct WireAssistantToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: WireAssistantToolCallFunction,
}

#[derive(Debug, Serialize)]
struct WireAssistantToolCallFunction {
    name: String,
    /// OpenAI expects a JSON *string*, not an object.
    arguments: String,
}

#[derive(Debug, Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireTool>>,
}

#[derive(Debug, Deserialize)]
struct WireChoice {
    message: WireChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireChoiceMessage {
    content: Option<String>,
    tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Debug, Deserialize)]
struct WireUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct WireResponse {
    choices: Option<Vec<WireChoice>>,
    usage: Option<WireUsage>,
}

// ── Wire-request builder (also exposed for tests) ─────────────────────────────

fn build_wire_messages(request: &LlmRequest) -> Vec<WireMessage> {
    let mut messages: Vec<WireMessage> = Vec::new();

    if let Some(sys) = &request.system_prompt {
        messages.push(WireMessage::Plain {
            role: "system".into(),
            content: sys.clone(),
        });
    }

    for msg in &request.messages {
        if let Some(tool_results) = &msg.tool_results {
            // Expand each tool result into a separate role: "tool" message.
            for result in tool_results {
                // OpenAI tool messages have no native error flag; prefix content with "ERROR: "
                // so the model can see the failure reason.
                let content = if result.is_error {
                    format!("ERROR: {}", result.content)
                } else {
                    result.content.clone()
                };
                messages.push(WireMessage::Tool {
                    role: "tool".into(),
                    tool_call_id: result.tool_use_id.clone(),
                    content,
                });
            }
        } else if let Some(tool_calls) = &msg.tool_calls {
            // Assistant message with tool calls.
            let wire_calls: Vec<WireAssistantToolCall> = tool_calls
                .iter()
                .map(|tc| WireAssistantToolCall {
                    id: tc.id.clone(),
                    kind: "function".into(),
                    function: WireAssistantToolCallFunction {
                        name: tc.name.clone(),
                        arguments: tc.input.to_string(),
                    },
                })
                .collect();
            messages.push(WireMessage::AssistantWithTools {
                role: msg.role.clone(),
                content: msg.content.clone(),
                tool_calls: wire_calls,
            });
        } else {
            messages.push(WireMessage::Plain {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }
    }

    messages
}

fn build_wire_request(request: &LlmRequest) -> WireRequest {
    let messages = build_wire_messages(request);
    let tools = request
        .tools
        .as_ref()
        .map(|ts| ts.iter().map(WireTool::from_llm_tool).collect::<Vec<_>>());

    WireRequest {
        model: request.model.clone(),
        messages,
        temperature: request.temperature,
        max_tokens: request.max_tokens,
        tools,
    }
}

/// Build the JSON that would be sent to the OpenAI API for a given request.
///
/// Exposed publicly so that unit tests can inspect the wire format without
/// making real HTTP calls.
#[cfg(any(test, feature = "test-utils"))]
pub fn build_wire_request_json(request: &LlmRequest) -> Value {
    serde_json::to_value(build_wire_request(request))
        .expect("WireRequest serialisation must not fail")
}

fn parse_tool_calls(wire_calls: Vec<WireToolCall>) -> Vec<ToolCall> {
    wire_calls
        .into_iter()
        .filter_map(|wc| {
            let id = wc.id?;
            let func = wc.function?;
            let name = func.name?;
            let args_str = func.arguments.unwrap_or_else(|| "{}".into());
            let input: Value = serde_json::from_str(&args_str).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse tool call arguments as JSON: {e}. Raw: {args_str}");
                Value::Null
            });
            Some(ToolCall { id, name, input })
        })
        .collect()
}

fn normalize_finish_reason(raw: &str) -> String {
    if raw == "tool_calls" {
        "tool_use".into()
    } else {
        raw.into()
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

pub struct OpenAiProviderBuilder {
    api_key: String,
    model: String,
    base_url: String,
    timeout_secs: u64,
}

impl OpenAiProviderBuilder {
    fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_secs: REQUEST_TIMEOUT_SECS,
        }
    }

    /// Override the API base URL.
    ///
    /// Use this to point to:
    /// - Azure OpenAI: `https://<resource>.openai.azure.com/openai/deployments/<deployment>`
    /// - ByteDance Doubao: `https://ark.cn-beijing.volces.com/api/v3`
    /// - Ollama local: `http://localhost:11434/v1`
    /// - Any OpenAI-compatible endpoint
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn build(self) -> OpenAiProvider {
        OpenAiProvider {
            api_key: self.api_key,
            model: self.model,
            base_url: self.base_url.trim_end_matches('/').to_string(),
            client: Client::builder()
                .timeout(Duration::from_secs(self.timeout_secs))
                .build()
                .expect("failed to build reqwest client"),
        }
    }
}

// ── OpenAiProvider ────────────────────────────────────────────────────────────

/// `LlmProvider` implementation for OpenAI-compatible chat completion APIs.
///
/// Works with:
/// - OpenAI (default: `https://api.openai.com/v1`)
/// - Azure OpenAI
/// - ByteDance Doubao (`https://ark.cn-beijing.volces.com/api/v3`)
/// - Ollama (`http://localhost:11434/v1`)
/// - Any endpoint that implements `POST /chat/completions`
pub struct OpenAiProvider {
    api_key: String,
    model: String,
    base_url: String,
    client: Client,
}

impl OpenAiProvider {
    /// Create with default OpenAI base URL.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::builder(api_key, model).build()
    }

    /// Create a builder for full configuration.
    pub fn builder(api_key: impl Into<String>, model: impl Into<String>) -> OpenAiProviderBuilder {
        OpenAiProviderBuilder::new(api_key, model)
    }

    /// The configured base URL (trailing slash stripped).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The configured model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The API key (used by submodules).
    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    /// The reqwest client (used by submodules).
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_wire_request(&request);

        debug!(
            "OpenAI request: model={}, url={}, msgs={}",
            body.model,
            url,
            body.messages.len()
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("LLM API HTTP {}: {}", status.as_u16(), text));
        }

        let wire: WireResponse = response.json().await?;

        let choices = wire.choices.unwrap_or_default();
        if choices.is_empty() {
            return Err(anyhow!("LLM API returned empty choices"));
        }

        let choice = &choices[0];

        // Content may be null when the model only returns tool calls.
        let content = choice.message.content.clone().unwrap_or_default();

        let tool_calls = choice
            .message
            .tool_calls
            .clone()
            .map(parse_tool_calls)
            .filter(|v: &Vec<ToolCall>| !v.is_empty());

        let finish_reason =
            normalize_finish_reason(choice.finish_reason.as_deref().unwrap_or("stop"));

        let usage = match wire.usage {
            Some(u) => LlmUsage {
                prompt_tokens: u.prompt_tokens.unwrap_or(0),
                completion_tokens: u.completion_tokens.unwrap_or(0),
            },
            None => LlmUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
            },
        };

        info!(
            "OpenAI response: model={}, prompt_tokens={}, completion_tokens={}, finish={}",
            request.model, usage.prompt_tokens, usage.completion_tokens, finish_reason
        );

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            finish_reason,
            tool_calls: None,
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use workflow_engine::llm::{LlmMessage, LlmTool, ToolCall, ToolResult};

    #[test]
    fn normalize_finish_reason_maps_tool_calls_to_tool_use() {
        assert_eq!(normalize_finish_reason("tool_calls"), "tool_use");
    }

    #[test]
    fn normalize_finish_reason_leaves_stop_unchanged() {
        assert_eq!(normalize_finish_reason("stop"), "stop");
    }

    #[test]
    fn normalize_finish_reason_leaves_end_turn_unchanged() {
        assert_eq!(normalize_finish_reason("end_turn"), "end_turn");
    }

    #[test]
    fn wire_request_includes_tools_field() {
        use serde_json::json;

        let tool = LlmTool {
            name: "search".into(),
            description: "Search the internet".into(),
            input_schema: json!({ "type": "object" }),
        };
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system_prompt: None,
            messages: vec![LlmMessage::user("hello")],
            temperature: None,
            max_tokens: None,
            tools: Some(vec![tool]),
        };
        let wire = build_wire_request(&req);
        let v = serde_json::to_value(&wire).unwrap();
        assert!(v["tools"].is_array());
        assert_eq!(v["tools"][0]["type"], "function");
        assert_eq!(v["tools"][0]["function"]["name"], "search");
    }

    #[test]
    fn wire_request_omits_tools_when_none() {
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system_prompt: None,
            messages: vec![LlmMessage::user("hello")],
            temperature: None,
            max_tokens: None,
            tools: None,
        };
        let wire = build_wire_request(&req);
        let v = serde_json::to_value(&wire).unwrap();
        assert!(v.get("tools").is_none());
    }

    #[test]
    fn tool_results_expand_to_role_tool_messages() {
        use serde_json::json;

        let req = LlmRequest {
            model: "gpt-4o".into(),
            system_prompt: None,
            messages: vec![
                LlmMessage::user("What's the weather?"),
                LlmMessage::assistant_with_tool_calls(
                    "",
                    vec![ToolCall {
                        id: "c1".into(),
                        name: "weather".into(),
                        input: json!({"city": "Berlin"}),
                    }],
                ),
                LlmMessage::tool_results(vec![ToolResult {
                    tool_use_id: "c1".into(),
                    content: "Rainy".into(),
                    is_error: false,
                }]),
            ],
            temperature: None,
            max_tokens: None,
            tools: None,
        };
        let msgs = build_wire_messages(&req);
        let v = serde_json::to_value(&msgs).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[2]["role"], "tool");
        assert_eq!(arr[2]["tool_call_id"], "c1");
        assert_eq!(arr[2]["content"], "Rainy");
    }

    #[test]
    fn parse_tool_calls_deserialises_arguments_string() {
        let raw = r#"[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"London\"}"}}]"#;
        let wire_calls: Vec<WireToolCall> = serde_json::from_str(raw).unwrap();
        let calls = parse_tool_calls(wire_calls);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].input["city"], "London");
    }

    #[test]
    fn error_tool_result_prefixes_content_with_error() {
        use serde_json::json;

        let req = LlmRequest {
            model: "gpt-4o".into(),
            system_prompt: None,
            messages: vec![
                LlmMessage::user("run something"),
                LlmMessage::assistant_with_tool_calls(
                    "",
                    vec![ToolCall {
                        id: "c1".into(),
                        name: "run_cmd".into(),
                        input: json!({}),
                    }],
                ),
                LlmMessage::tool_results(vec![ToolResult {
                    tool_use_id: "c1".into(),
                    content: "command not found".into(),
                    is_error: true,
                }]),
            ],
            temperature: None,
            max_tokens: None,
            tools: None,
        };
        let msgs = build_wire_messages(&req);
        let v = serde_json::to_value(&msgs).unwrap();
        let arr = v.as_array().unwrap();
        // user + assistant + tool = 3
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[2]["role"], "tool");
        assert_eq!(arr[2]["content"], "ERROR: command not found");
    }

    #[test]
    fn non_error_tool_result_content_unchanged() {
        use serde_json::json;

        let req = LlmRequest {
            model: "gpt-4o".into(),
            system_prompt: None,
            messages: vec![
                LlmMessage::user("search"),
                LlmMessage::assistant_with_tool_calls(
                    "",
                    vec![ToolCall {
                        id: "c2".into(),
                        name: "search".into(),
                        input: json!({}),
                    }],
                ),
                LlmMessage::tool_results(vec![ToolResult {
                    tool_use_id: "c2".into(),
                    content: "42 results".into(),
                    is_error: false,
                }]),
            ],
            temperature: None,
            max_tokens: None,
            tools: None,
        };
        let msgs = build_wire_messages(&req);
        let v = serde_json::to_value(&msgs).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr[2]["content"], "42 results");
    }

    #[test]
    fn parse_tool_calls_returns_null_for_invalid_json_arguments() {
        // Invalid JSON in arguments — should not panic, falls back to Null.
        let raw = r#"[{"id":"call_bad","type":"function","function":{"name":"foo","arguments":"not-valid-json"}}]"#;
        let wire_calls: Vec<WireToolCall> = serde_json::from_str(raw).unwrap();
        let calls = parse_tool_calls(wire_calls);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_bad");
        assert_eq!(calls[0].input, Value::Null);
    }
}
