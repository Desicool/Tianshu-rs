// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use workflow_engine::llm::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmUsage, ToolCall};
use workflow_engine::observe::{Observer, ToolCallRecord};
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

// ── Observer per-tool timing test (TDD for duration_ms fix) ──────────────────

struct RecordingObserver {
    tool_records: Arc<Mutex<Vec<ToolCallRecord>>>,
}

#[async_trait]
impl Observer for RecordingObserver {
    async fn on_tool_call(&self, record: &ToolCallRecord) {
        self.tool_records.lock().unwrap().push(record.clone());
    }
}

struct SlowTool {
    delay_ms: u64,
}

#[async_trait]
impl Tool for SlowTool {
    fn name(&self) -> &str {
        "slow_tool"
    }
    fn description(&self) -> &str {
        "Sleeps for a bit"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::ReadOnly
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({ "type": "object" })
    }
    async fn execute(&self, _input: JsonValue) -> Result<String> {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        Ok("done".into())
    }
}

struct InstantTool;

#[async_trait]
impl Tool for InstantTool {
    fn name(&self) -> &str {
        "instant_tool"
    }
    fn description(&self) -> &str {
        "Returns instantly"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::ReadOnly
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({ "type": "object" })
    }
    async fn execute(&self, _input: JsonValue) -> Result<String> {
        Ok("instant".into())
    }
}

#[tokio::test]
async fn tool_loop_observer_records_per_tool_duration_not_batch_time() {
    // The LLM requests two concurrent ReadOnly tools in round 1: one instant, one slow (20ms).
    // After the fix, the observer should see distinct per-tool durations.
    // With the bug, both records would show the same batch wall time (~20ms).
    let slow_call = ToolCall {
        id: "slow_id".into(),
        name: "slow_tool".into(),
        arguments: "{}".into(),
    };
    let fast_call = ToolCall {
        id: "fast_id".into(),
        name: "instant_tool".into(),
        arguments: "{}".into(),
    };

    let llm = MockLlm::new(vec![
        // Round 1: request both tools (concurrent batch)
        tool_call_response(vec![fast_call, slow_call]),
        // Round 2: LLM returns final text
        text_response("all done"),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(SlowTool { delay_ms: 20 });
    registry.register(InstantTool);

    let recorded = Arc::new(Mutex::new(Vec::new()));
    let observer = RecordingObserver {
        tool_records: recorded.clone(),
    };

    let config = ToolLoopConfig::default();
    let _result = run_tool_loop(&llm, make_request(), &registry, &config, Some(&observer))
        .await
        .unwrap();

    let records = recorded.lock().unwrap().clone();
    assert_eq!(records.len(), 2, "expected 2 ToolCallRecords");

    let fast_rec = records.iter().find(|r| r.call_id == "fast_id").unwrap();
    let slow_rec = records.iter().find(|r| r.call_id == "slow_id").unwrap();

    // Per-tool durations must differ: fast should be strictly less than slow.
    // With the batch-time bug both would equal ~20ms and this assertion fails.
    assert!(
        fast_rec.duration_ms < slow_rec.duration_ms,
        "fast tool duration ({}ms) should be < slow tool duration ({}ms)",
        fast_rec.duration_ms,
        slow_rec.duration_ms
    );
    assert!(
        slow_rec.duration_ms >= 15,
        "slow tool should report ~20ms, got {}ms",
        slow_rec.duration_ms
    );
}