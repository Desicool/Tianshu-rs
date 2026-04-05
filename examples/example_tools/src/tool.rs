use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use workflow_engine::llm::{LlmMessage, LlmProvider, LlmRequest, LlmTool, ToolCall, ToolResult};

/// Whether a tool is safe to run concurrently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSafety {
    /// Read-only; can run in parallel via JoinSet.
    ConcurrentSafe,
    /// Write/destructive; must run sequentially.
    Exclusive,
}

/// A tool that can be called by an LLM.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn safety(&self) -> ToolSafety;
    /// Returns a JSON Schema `{"type":"object","properties":{...},...}`.
    fn input_schema(&self) -> JsonValue;
    async fn execute(&self, input: JsonValue) -> Result<String>;
}

/// Registry of tools available to an LLM agent.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Any previous tool with the same name is replaced.
    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Convert all registered tools to `LlmTool` definitions for API requests.
    pub fn to_llm_tools(&self) -> Vec<LlmTool> {
        self.tools
            .values()
            .map(|t| LlmTool {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    /// Execute a single tool call, returning a `ToolResult` (never panics).
    pub async fn execute_call(&self, call: &ToolCall) -> ToolResult {
        match self.tools.get(&call.name) {
            None => ToolResult {
                tool_use_id: call.id.clone(),
                content: format!("Unknown tool: {}", call.name),
                is_error: true,
            },
            Some(tool) => match tool.execute(call.input.clone()).await {
                Ok(output) => ToolResult {
                    tool_use_id: call.id.clone(),
                    content: output,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    tool_use_id: call.id.clone(),
                    content: format!("Tool error: {e}"),
                    is_error: true,
                },
            },
        }
    }

    /// Execute multiple tool calls concurrently (ConcurrentSafe tools only).
    ///
    /// The caller is responsible for ensuring all calls are safe to run in
    /// parallel. Results are returned in the same order as the input slice.
    pub async fn execute_calls_parallel(&self, calls: &[ToolCall]) -> Vec<ToolResult> {
        use tokio::task::JoinSet;

        let mut set: JoinSet<(usize, ToolResult)> = JoinSet::new();

        for (idx, call) in calls.iter().enumerate() {
            let tool = match self.tools.get(&call.name) {
                Some(t) => t.clone(),
                None => {
                    // Return an error result inline without spawning.
                    let result = ToolResult {
                        tool_use_id: call.id.clone(),
                        content: format!("Unknown tool: {}", call.name),
                        is_error: true,
                    };
                    set.spawn(async move { (idx, result) });
                    continue;
                }
            };
            let call = call.clone();
            set.spawn(async move {
                let result = match tool.execute(call.input.clone()).await {
                    Ok(output) => ToolResult {
                        tool_use_id: call.id.clone(),
                        content: output,
                        is_error: false,
                    },
                    Err(e) => ToolResult {
                        tool_use_id: call.id.clone(),
                        content: format!("Tool error: {e}"),
                        is_error: true,
                    },
                };
                (idx, result)
            });
        }

        let mut indexed: Vec<(usize, ToolResult)> = Vec::with_capacity(calls.len());
        while let Some(res) = set.join_next().await {
            // JoinError means panic in task — propagate as error result.
            match res {
                Ok(pair) => indexed.push(pair),
                Err(e) => {
                    // We can't recover the original index here easily, so push at end.
                    indexed.push((
                        calls.len(),
                        ToolResult {
                            tool_use_id: String::new(),
                            content: format!("Task panicked: {e}"),
                            is_error: true,
                        },
                    ));
                }
            }
        }

        indexed.sort_by_key(|(idx, _)| *idx);
        indexed.into_iter().map(|(_, r)| r).collect()
    }
}

/// Run the LLM tool-use loop until the model returns a non-tool response or
/// `max_tool_rounds` is exhausted.
///
/// Returns `(final_text, message_history)`.
pub async fn run_tool_loop(
    llm: &dyn LlmProvider,
    mut request: LlmRequest,
    registry: &ToolRegistry,
    max_tool_rounds: usize,
) -> Result<(String, Vec<LlmMessage>)> {
    for _round in 0..=max_tool_rounds {
        let response = llm.complete(request.clone()).await?;

        if response.finish_reason != "tool_use" {
            return Ok((response.content, request.messages));
        }

        // We hit the round cap — stop without executing tools.
        if _round == max_tool_rounds {
            return Ok((response.content, request.messages));
        }

        let tool_calls = response.tool_calls.unwrap_or_default();

        // Partition calls by safety.
        let (safe_calls, exclusive_calls): (Vec<_>, Vec<_>) =
            tool_calls.iter().cloned().partition(|c| {
                registry
                    .get(&c.name)
                    .map(|t| t.safety() == ToolSafety::ConcurrentSafe)
                    .unwrap_or(false) // unknown tools treated as exclusive
            });

        // Execute concurrently-safe calls in parallel.
        let safe_results = registry.execute_calls_parallel(&safe_calls).await;

        // Execute exclusive calls sequentially.
        let mut exclusive_results = Vec::with_capacity(exclusive_calls.len());
        for call in &exclusive_calls {
            exclusive_results.push(registry.execute_call(call).await);
        }

        // Merge results back in original tool_calls order.
        let mut safe_iter = safe_results.into_iter();
        let mut excl_iter = exclusive_results.into_iter();
        let mut results = Vec::with_capacity(tool_calls.len());
        for call in &tool_calls {
            let safety = registry
                .get(&call.name)
                .map(|t| t.safety())
                .unwrap_or(ToolSafety::Exclusive);
            let result = if safety == ToolSafety::ConcurrentSafe {
                safe_iter.next().expect("safe result missing")
            } else {
                excl_iter.next().expect("exclusive result missing")
            };
            results.push(result);
        }

        // Append assistant message with tool calls.
        request
            .messages
            .push(LlmMessage::assistant_with_tool_calls(
                response.content,
                tool_calls,
            ));
        // Append tool results.
        request.messages.push(LlmMessage::tool_results(results));
    }

    // Unreachable in normal usage but satisfies the compiler.
    Ok((String::new(), request.messages))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;
    use workflow_engine::llm::{LlmResponse, LlmUsage};

    // ── Mock helpers ─────────────────────────────────────────────────────────

    /// A simple tool that always returns a fixed string.
    struct EchoTool {
        name: &'static str,
        safety: ToolSafety,
        reply: &'static str,
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "Echo tool"
        }
        fn safety(&self) -> ToolSafety {
            self.safety
        }
        fn input_schema(&self) -> JsonValue {
            json!({"type":"object","properties":{}})
        }
        async fn execute(&self, _input: JsonValue) -> Result<String> {
            Ok(self.reply.to_string())
        }
    }

    /// A tool that always returns an error.
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
            ToolSafety::Exclusive
        }
        fn input_schema(&self) -> JsonValue {
            json!({"type":"object","properties":{}})
        }
        async fn execute(&self, _input: JsonValue) -> Result<String> {
            Err(anyhow::anyhow!("intentional error"))
        }
    }

    /// Mock LLM that returns canned responses in sequence.
    struct MockLlmProvider {
        responses: std::sync::Mutex<VecDeque<LlmResponse>>,
    }

    impl MockLlmProvider {
        fn new(responses: Vec<LlmResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlmProvider {
        async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse> {
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("no more canned responses"))
        }
    }

    fn stop_response(content: &str) -> LlmResponse {
        LlmResponse {
            content: content.to_string(),
            tool_calls: None,
            usage: LlmUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
            },
            finish_reason: "stop".to_string(),
        }
    }

    fn tool_use_response(tool_name: &str, call_id: &str, input: JsonValue) -> LlmResponse {
        LlmResponse {
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                input,
            }]),
            usage: LlmUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
            },
            finish_reason: "tool_use".to_string(),
        }
    }

    fn make_request() -> LlmRequest {
        LlmRequest {
            model: "mock".to_string(),
            system_prompt: None,
            messages: vec![LlmMessage::user("do something")],
            temperature: None,
            max_tokens: None,
            tools: None,
        }
    }

    // ── ToolRegistry tests ────────────────────────────────────────────────────

    #[test]
    fn registry_new_creates_empty() {
        let reg = ToolRegistry::new();
        assert!(reg.tools.is_empty());
    }

    #[test]
    fn register_then_get_returns_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool {
            name: "echo",
            safety: ToolSafety::ConcurrentSafe,
            reply: "hi",
        });
        let tool = reg.get("echo").expect("tool should be found");
        assert_eq!(tool.name(), "echo");
    }

    #[test]
    fn to_llm_tools_returns_correct_fields() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool {
            name: "my_tool",
            safety: ToolSafety::ConcurrentSafe,
            reply: "ok",
        });
        let llm_tools = reg.to_llm_tools();
        assert_eq!(llm_tools.len(), 1);
        assert_eq!(llm_tools[0].name, "my_tool");
        assert_eq!(llm_tools[0].description, "Echo tool");
        assert_eq!(llm_tools[0].input_schema["type"], "object");
    }

    #[tokio::test]
    async fn execute_call_unknown_tool_returns_error() {
        let reg = ToolRegistry::new();
        let call = ToolCall {
            id: "c1".to_string(),
            name: "nonexistent".to_string(),
            input: json!({}),
        };
        let result = reg.execute_call(&call).await;
        assert_eq!(result.tool_use_id, "c1");
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn execute_call_success_returns_ok_result() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool {
            name: "echo",
            safety: ToolSafety::ConcurrentSafe,
            reply: "pong",
        });
        let call = ToolCall {
            id: "c2".to_string(),
            name: "echo".to_string(),
            input: json!({}),
        };
        let result = reg.execute_call(&call).await;
        assert_eq!(result.tool_use_id, "c2");
        assert!(!result.is_error);
        assert_eq!(result.content, "pong");
    }

    #[tokio::test]
    async fn execute_call_error_tool_returns_is_error_true() {
        let mut reg = ToolRegistry::new();
        reg.register(ErrorTool);
        let call = ToolCall {
            id: "c3".to_string(),
            name: "error_tool".to_string(),
            input: json!({}),
        };
        let result = reg.execute_call(&call).await;
        assert_eq!(result.tool_use_id, "c3");
        assert!(result.is_error);
        assert!(result.content.contains("intentional error"));
    }

    #[tokio::test]
    async fn execute_calls_parallel_runs_all() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool {
            name: "t1",
            safety: ToolSafety::ConcurrentSafe,
            reply: "result1",
        });
        reg.register(EchoTool {
            name: "t2",
            safety: ToolSafety::ConcurrentSafe,
            reply: "result2",
        });
        let calls = vec![
            ToolCall {
                id: "a".to_string(),
                name: "t1".to_string(),
                input: json!({}),
            },
            ToolCall {
                id: "b".to_string(),
                name: "t2".to_string(),
                input: json!({}),
            },
        ];
        let results = reg.execute_calls_parallel(&calls).await;
        assert_eq!(results.len(), 2);
        // Order must match input.
        assert_eq!(results[0].tool_use_id, "a");
        assert_eq!(results[0].content, "result1");
        assert_eq!(results[1].tool_use_id, "b");
        assert_eq!(results[1].content, "result2");
    }

    // ── run_tool_loop tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn run_tool_loop_stop_immediately_no_tools_called() {
        let llm = MockLlmProvider::new(vec![stop_response("Hello world")]);
        let reg = ToolRegistry::new();
        let (text, messages) = run_tool_loop(&llm, make_request(), &reg, 3)
            .await
            .unwrap();
        assert_eq!(text, "Hello world");
        // No tool messages appended — just the original user message.
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
    }

    #[tokio::test]
    async fn run_tool_loop_tool_use_once_then_stop() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool {
            name: "echo",
            safety: ToolSafety::ConcurrentSafe,
            reply: "tool_output",
        });

        let llm = MockLlmProvider::new(vec![
            tool_use_response("echo", "call_1", json!({})),
            stop_response("Final answer"),
        ]);

        let (text, messages) = run_tool_loop(&llm, make_request(), &reg, 3)
            .await
            .unwrap();

        assert_eq!(text, "Final answer");
        // user + assistant-with-tool-calls + tool-results = 3 messages
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, "assistant");
        assert!(messages[1].tool_calls.is_some());
        assert_eq!(messages[2].role, "user");
        assert!(messages[2].tool_results.is_some());
        let results = messages[2].tool_results.as_ref().unwrap();
        assert_eq!(results[0].content, "tool_output");
    }

    #[tokio::test]
    async fn run_tool_loop_stops_at_max_rounds() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool {
            name: "echo",
            safety: ToolSafety::ConcurrentSafe,
            reply: "ok",
        });

        // LLM always returns tool_use — should stop after max_tool_rounds.
        let responses = vec![
            tool_use_response("echo", "c1", json!({})),
            tool_use_response("echo", "c2", json!({})),
            tool_use_response("echo", "c3", json!({})),
        ];
        let llm = MockLlmProvider::new(responses);
        let max = 2;
        let (text, _messages) = run_tool_loop(&llm, make_request(), &reg, max)
            .await
            .unwrap();
        // After max_tool_rounds the last response content is returned.
        // The loop executes tools for rounds 0 and 1, then on round 2 (== max)
        // it returns without executing tools.
        let _ = text; // just assert it completes without error
    }
}
