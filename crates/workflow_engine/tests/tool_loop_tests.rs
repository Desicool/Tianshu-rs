use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};
use std::collections::VecDeque;
use std::sync::Mutex;

use workflow_engine::llm::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmUsage, ToolCall};
use workflow_engine::tool::{Tool, ToolRegistry, ToolSafety};
use workflow_engine::tool_loop::{run_tool_loop, ToolLoopConfig};

// ── Mock LLM ─────────────────────────────────────────────────────────────────

struct MockLlm {
    responses: Mutex<VecDeque<LlmResponse>>,
}

impl MockLlm {
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }
}

#[async_trait]
impl LlmProvider for MockLlm {
    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse> {
        let resp = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("MockLlm: no more responses queued"))?;
        Ok(resp)
    }
}

fn text_response(text: &str) -> LlmResponse {
    LlmResponse {
        content: text.into(),
        usage: LlmUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
        },
        finish_reason: "stop".into(),
        tool_calls: None,
    }
}

fn tool_call_response(calls: Vec<ToolCall>) -> LlmResponse {
    LlmResponse {
        content: String::new(),
        usage: LlmUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
        },
        finish_reason: "tool_calls".into(),
        tool_calls: Some(calls),
    }
}

// ── Test tool ────────────────────────────────────────────────────────────────

struct AddTool;

#[async_trait]
impl Tool for AddTool {
    fn name(&self) -> &str {
        "add"
    }
    fn description(&self) -> &str {
        "Adds two numbers"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::ReadOnly
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "a": { "type": "number" },
                "b": { "type": "number" }
            },
            "required": ["a", "b"]
        })
    }
    async fn execute(&self, input: JsonValue) -> Result<String> {
        let a = input["a"].as_f64().unwrap_or(0.0);
        let b = input["b"].as_f64().unwrap_or(0.0);
        Ok(format!("{}", a + b))
    }
}

struct ErrorTool;

#[async_trait]
impl Tool for ErrorTool {
    fn name(&self) -> &str {
        "error_tool"
    }
    fn description(&self) -> &str {
        "Always errors"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::ReadOnly
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({ "type": "object" })
    }
    async fn execute(&self, _input: JsonValue) -> Result<String> {
        Err(anyhow::anyhow!("something went wrong"))
    }
}

fn make_request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        system_prompt: Some("You are helpful".into()),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "What is 2+3?".into(),
            tool_calls: None,
            tool_call_id: None,
        }],
        temperature: Some(0.0),
        max_tokens: Some(100),
        tools: None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tool_loop_no_tools_returns_text() {
    let llm = MockLlm::new(vec![text_response("The answer is 5")]);

    let mut registry = ToolRegistry::new();
    registry.register(AddTool);

    let config = ToolLoopConfig::default();
    let result = run_tool_loop(&llm, make_request(), &registry, &config, None)
        .await
        .unwrap();

    assert_eq!(result.final_text, "The answer is 5");
    assert_eq!(result.rounds, 1);
    assert_eq!(result.total_tool_calls, 0);
}

#[tokio::test]
async fn tool_loop_single_tool_call() {
    let llm = MockLlm::new(vec![
        // Round 1: LLM requests a tool call
        tool_call_response(vec![ToolCall {
            id: "call_1".into(),
            name: "add".into(),
            arguments: r#"{"a": 2, "b": 3}"#.into(),
        }]),
        // Round 2: LLM returns final text after seeing tool result
        text_response("The answer is 5"),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(AddTool);

    let config = ToolLoopConfig::default();
    let result = run_tool_loop(&llm, make_request(), &registry, &config, None)
        .await
        .unwrap();

    assert_eq!(result.final_text, "The answer is 5");
    assert_eq!(result.rounds, 2);
    assert_eq!(result.total_tool_calls, 1);

    // Verify message history includes tool result
    let tool_messages: Vec<&LlmMessage> = result
        .messages
        .iter()
        .filter(|m| m.role == "tool")
        .collect();
    assert_eq!(tool_messages.len(), 1);
    assert_eq!(tool_messages[0].content, "5"); // 2 + 3
    assert_eq!(tool_messages[0].tool_call_id.as_deref(), Some("call_1"));
}

#[tokio::test]
async fn tool_loop_max_rounds_exceeded() {
    // LLM always returns tool calls, never text
    let responses: Vec<LlmResponse> = (0..5)
        .map(|i| {
            tool_call_response(vec![ToolCall {
                id: format!("call_{i}"),
                name: "add".into(),
                arguments: r#"{"a": 1, "b": 1}"#.into(),
            }])
        })
        .collect();

    let llm = MockLlm::new(responses);

    let mut registry = ToolRegistry::new();
    registry.register(AddTool);

    let config = ToolLoopConfig {
        max_rounds: 3,
        max_concurrency: 5,
    };

    let result = run_tool_loop(&llm, make_request(), &registry, &config, None).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("max_rounds"),
        "Error should mention max_rounds, got: {err_msg}"
    );
}

#[tokio::test]
async fn tool_loop_tool_error_continues() {
    let llm = MockLlm::new(vec![
        // Round 1: LLM calls the error tool
        tool_call_response(vec![ToolCall {
            id: "call_err".into(),
            name: "error_tool".into(),
            arguments: "{}".into(),
        }]),
        // Round 2: LLM gets error result and returns text
        text_response("Sorry, that tool failed"),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(ErrorTool);

    let config = ToolLoopConfig::default();
    let result = run_tool_loop(&llm, make_request(), &registry, &config, None)
        .await
        .unwrap();

    assert_eq!(result.final_text, "Sorry, that tool failed");
    assert_eq!(result.rounds, 2);
    assert_eq!(result.total_tool_calls, 1);

    // The tool error message should be in the messages
    let tool_messages: Vec<&LlmMessage> = result
        .messages
        .iter()
        .filter(|m| m.role == "tool")
        .collect();
    assert_eq!(tool_messages.len(), 1);
    assert!(tool_messages[0].content.contains("something went wrong"));
}
