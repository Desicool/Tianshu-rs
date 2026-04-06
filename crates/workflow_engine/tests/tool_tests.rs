// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};
use workflow_engine::llm::ToolCall;
use workflow_engine::tool::{Tool, ToolRegistry, ToolSafety};

// ── Test tools ──────────────────────────────────────────────────────────────

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Echoes input back"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::ReadOnly
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            },
            "required": ["text"]
        })
    }
    async fn execute(&self, input: JsonValue) -> Result<String> {
        let text = input["text"].as_str().unwrap_or("no text");
        Ok(format!("echo: {text}"))
    }
}

struct FailTool;

#[async_trait]
impl Tool for FailTool {
    fn name(&self) -> &str {
        "fail"
    }
    fn description(&self) -> &str {
        "Always fails"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::Exclusive
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({ "type": "object" })
    }
    async fn execute(&self, _input: JsonValue) -> Result<String> {
        Err(anyhow::anyhow!("tool failed intentionally"))
    }
}

struct SlowReadOnlyTool {
    name: String,
    delay_ms: u64,
}

#[async_trait]
impl Tool for SlowReadOnlyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Slow read-only tool"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::ReadOnly
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({ "type": "object" })
    }
    async fn execute(&self, _input: JsonValue) -> Result<String> {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        Ok(format!("{} done", self.name))
    }
}

struct SlowExclusiveTool;

#[async_trait]
impl Tool for SlowExclusiveTool {
    fn name(&self) -> &str {
        "exclusive_slow"
    }
    fn description(&self) -> &str {
        "Slow exclusive tool"
    }
    fn safety(&self) -> ToolSafety {
        ToolSafety::Exclusive
    }
    fn parameters_schema(&self) -> JsonValue {
        json!({ "type": "object" })
    }
    async fn execute(&self, _input: JsonValue) -> Result<String> {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok("exclusive done".into())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn tool_registry_register_and_get() {
    let mut registry = ToolRegistry::new();
    registry.register(EchoTool);

    // Get registered tool by name
    let tool = registry.get("echo");
    assert!(tool.is_some());
    assert_eq!(tool.unwrap().name(), "echo");

    // Unknown tool returns None
    assert!(registry.get("unknown").is_none());
}

#[test]
fn tool_registry_to_llm_tools() {
    let mut registry = ToolRegistry::new();
    registry.register(EchoTool);
    registry.register(FailTool);

    let llm_tools = registry.to_llm_tools();
    assert_eq!(llm_tools.len(), 2);

    let names: Vec<&str> = llm_tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"echo"));
    assert!(names.contains(&"fail"));

    // Check that descriptions and parameters are correct
    let echo_tool = llm_tools.iter().find(|t| t.name == "echo").unwrap();
    assert_eq!(echo_tool.description, "Echoes input back");
    assert!(echo_tool.parameters["properties"]["text"].is_object());
}

#[tokio::test]
async fn tool_registry_execute_call() {
    let mut registry = ToolRegistry::new();
    registry.register(EchoTool);

    let call = ToolCall {
        id: "call_1".into(),
        name: "echo".into(),
        arguments: r#"{"text": "hello"}"#.into(),
    };

    let result = registry.execute_call(&call).await;
    assert_eq!(result.call_id, "call_1");
    assert_eq!(result.content, "echo: hello");
    assert!(!result.is_error);
}

#[tokio::test]
async fn tool_registry_execute_call_error() {
    let mut registry = ToolRegistry::new();
    registry.register(FailTool);

    let call = ToolCall {
        id: "call_2".into(),
        name: "fail".into(),
        arguments: "{}".into(),
    };

    let result = registry.execute_call(&call).await;
    assert_eq!(result.call_id, "call_2");
    assert!(result.is_error);
    assert!(result.content.contains("tool failed intentionally"));
}

#[test]
fn tool_safety_variants() {
    assert_ne!(ToolSafety::ReadOnly, ToolSafety::Exclusive);
    assert_eq!(ToolSafety::ReadOnly, ToolSafety::ReadOnly);
    assert_eq!(ToolSafety::Exclusive, ToolSafety::Exclusive);
}

#[tokio::test]
async fn execute_with_concurrency_readonly_run_in_parallel() {
    let mut registry = ToolRegistry::new();
    registry.register(SlowReadOnlyTool {
        name: "slow_a".into(),
        delay_ms: 50,
    });
    registry.register(SlowReadOnlyTool {
        name: "slow_b".into(),
        delay_ms: 50,
    });

    let calls = vec![
        ToolCall {
            id: "c1".into(),
            name: "slow_a".into(),
            arguments: "{}".into(),
        },
        ToolCall {
            id: "c2".into(),
            name: "slow_b".into(),
            arguments: "{}".into(),
        },
    ];

    let start = std::time::Instant::now();
    let results = registry.execute_with_concurrency(&calls, 10).await;
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0.call_id, "c1");
    assert_eq!(results[1].0.call_id, "c2");
    assert!(!results[0].0.is_error);
    assert!(!results[1].0.is_error);

    // Both ReadOnly tools should run in parallel, so total time < 2x single tool time
    assert!(
        elapsed.as_millis() < 90,
        "ReadOnly tools should run in parallel, took {}ms",
        elapsed.as_millis()
    );
}

#[tokio::test]
async fn execute_with_concurrency_exclusive_runs_alone() {
    let mut registry = ToolRegistry::new();
    registry.register(SlowReadOnlyTool {
        name: "slow_a".into(),
        delay_ms: 50,
    });
    registry.register(SlowExclusiveTool);

    // ReadOnly then Exclusive — they should run in separate batches
    let calls = vec![
        ToolCall {
            id: "c1".into(),
            name: "slow_a".into(),
            arguments: "{}".into(),
        },
        ToolCall {
            id: "c2".into(),
            name: "exclusive_slow".into(),
            arguments: "{}".into(),
        },
    ];

    let results = registry.execute_with_concurrency(&calls, 10).await;

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0.call_id, "c1");
    assert_eq!(results[0].0.content, "slow_a done");
    assert_eq!(results[1].0.call_id, "c2");
    assert_eq!(results[1].0.content, "exclusive done");
}

#[tokio::test]
async fn execute_call_unknown_tool_returns_error() {
    let registry = ToolRegistry::new();

    let call = ToolCall {
        id: "call_x".into(),
        name: "nonexistent".into(),
        arguments: "{}".into(),
    };

    let result = registry.execute_call(&call).await;
    assert!(result.is_error);
    assert!(result.content.contains("nonexistent"));
}

// ── Per-tool timing tests (TDD for duration_ms fix) ──────────────────────────

#[tokio::test]
async fn execute_with_concurrency_returns_per_tool_duration() {
    // Fast tool (~0ms) and slow tool (20ms) run concurrently.
    // After the fix, each (ToolResult, duration_ms) pair should reflect that
    // tool's own execution time — not the full batch wall time.
    let mut registry = ToolRegistry::new();
    registry.register(SlowReadOnlyTool {
        name: "instant".into(),
        delay_ms: 0,
    });
    registry.register(SlowReadOnlyTool {
        name: "slow_20".into(),
        delay_ms: 20,
    });

    let calls = vec![
        ToolCall {
            id: "fast".into(),
            name: "instant".into(),
            arguments: "{}".into(),
        },
        ToolCall {
            id: "slow".into(),
            name: "slow_20".into(),
            arguments: "{}".into(),
        },
    ];

    // execute_with_concurrency now returns Vec<(ToolResult, u64)>
    let results: Vec<(workflow_engine::llm::ToolResult, u64)> =
        registry.execute_with_concurrency(&calls, 10).await;

    assert_eq!(results.len(), 2);
    let (fast_result, fast_ms) = &results[0];
    let (slow_result, slow_ms) = &results[1];

    assert_eq!(fast_result.call_id, "fast");
    assert_eq!(slow_result.call_id, "slow");
    assert!(!fast_result.is_error);
    assert!(!slow_result.is_error);

    // The slow tool must report a longer duration than the fast tool.
    // With batch-time stamping (the bug), both would be ~20ms and this fails.
    assert!(
        fast_ms < slow_ms,
        "fast tool duration ({fast_ms}ms) should be less than slow tool duration ({slow_ms}ms)"
    );
    // Slow tool should be at least 15ms (allowing for timer jitter)
    assert!(
        *slow_ms >= 15,
        "slow tool should report ~20ms duration, got {slow_ms}ms"
    );
}

#[tokio::test]
async fn execute_with_concurrency_exclusive_returns_duration() {
    // Exclusive tools should also carry their individual duration.
    let mut registry = ToolRegistry::new();
    registry.register(SlowExclusiveTool);

    let calls = vec![ToolCall {
        id: "excl".into(),
        name: "exclusive_slow".into(),
        arguments: "{}".into(),
    }];

    let results: Vec<(workflow_engine::llm::ToolResult, u64)> =
        registry.execute_with_concurrency(&calls, 10).await;

    assert_eq!(results.len(), 1);
    let (result, duration_ms) = &results[0];
    assert_eq!(result.call_id, "excl");
    assert!(!result.is_error);
    // SlowExclusiveTool sleeps 50ms
    assert!(
        *duration_ms >= 40,
        "exclusive tool should report ~50ms duration, got {duration_ms}ms"
    );
}