use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;

use crate::llm::{LlmTool, ToolCall, ToolResult};

/// Whether a tool is safe to run concurrently with other tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSafety {
    /// Tool performs only read operations; safe to run concurrently with other ReadOnly tools.
    ReadOnly,
    /// Tool mutates state; must run alone (no other tool calls in the same batch).
    Exclusive,
}

/// A tool that can be invoked by an LLM during a tool-use loop.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn safety(&self) -> ToolSafety;
    fn parameters_schema(&self) -> JsonValue;
    async fn execute(&self, input: JsonValue) -> Result<String>;
}

/// Registry of available tools, used to resolve and execute tool calls.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn to_llm_tools(&self) -> Vec<LlmTool> {
        self.tools
            .values()
            .map(|t| LlmTool {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect()
    }

    /// Execute a single tool call, returning a `ToolResult`.
    pub async fn execute_call(&self, call: &ToolCall) -> ToolResult {
        let tool = match self.get(&call.name) {
            Some(t) => t,
            None => {
                return ToolResult {
                    call_id: call.id.clone(),
                    content: format!("unknown tool: {}", call.name),
                    is_error: true,
                };
            }
        };

        let args: JsonValue = match serde_json::from_str(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult {
                    call_id: call.id.clone(),
                    content: format!("failed to parse arguments: {e}"),
                    is_error: true,
                };
            }
        };

        match tool.execute(args).await {
            Ok(content) => ToolResult {
                call_id: call.id.clone(),
                content,
                is_error: false,
            },
            Err(e) => ToolResult {
                call_id: call.id.clone(),
                content: e.to_string(),
                is_error: true,
            },
        }
    }

    /// Execute multiple tool calls respecting `ToolSafety`.
    ///
    /// Consecutive `ReadOnly` calls are batched and run in parallel (up to
    /// `max_concurrency`). `Exclusive` calls always run alone. Results are
    /// returned in the same order as the input `calls`.
    pub async fn execute_with_concurrency(
        &self,
        calls: &[ToolCall],
        max_concurrency: usize,
    ) -> Vec<ToolResult> {
        if calls.is_empty() {
            return vec![];
        }

        let mut results = Vec::with_capacity(calls.len());

        // Group consecutive calls by safety level
        let mut i = 0;
        while i < calls.len() {
            let call = &calls[i];
            let safety = self
                .get(&call.name)
                .map(|t| t.safety())
                .unwrap_or(ToolSafety::Exclusive);

            if safety == ToolSafety::Exclusive {
                // Run exclusive tool alone
                results.push(self.execute_call(call).await);
                i += 1;
            } else {
                // Gather consecutive ReadOnly calls
                let batch_start = i;
                while i < calls.len() {
                    let s = self
                        .get(&calls[i].name)
                        .map(|t| t.safety())
                        .unwrap_or(ToolSafety::Exclusive);
                    if s != ToolSafety::ReadOnly {
                        break;
                    }
                    i += 1;
                }
                let batch = &calls[batch_start..i];

                // Run batch in parallel using JoinSet, capped at max_concurrency
                let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrency));
                let mut handles = Vec::with_capacity(batch.len());

                for c in batch {
                    let sem = semaphore.clone();
                    let tool = self.get(&c.name);
                    let call_id = c.id.clone();
                    let arguments = c.arguments.clone();
                    let name = c.name.clone();

                    handles.push(tokio::spawn(async move {
                        let _permit = sem.acquire().await.unwrap();
                        let tool = match tool {
                            Some(t) => t,
                            None => {
                                return ToolResult {
                                    call_id,
                                    content: format!("unknown tool: {name}"),
                                    is_error: true,
                                };
                            }
                        };
                        let args: JsonValue = match serde_json::from_str(&arguments) {
                            Ok(v) => v,
                            Err(e) => {
                                return ToolResult {
                                    call_id,
                                    content: format!("failed to parse arguments: {e}"),
                                    is_error: true,
                                };
                            }
                        };
                        match tool.execute(args).await {
                            Ok(content) => ToolResult {
                                call_id,
                                content,
                                is_error: false,
                            },
                            Err(e) => ToolResult {
                                call_id,
                                content: e.to_string(),
                                is_error: true,
                            },
                        }
                    }));
                }

                // Collect results in order
                for handle in handles {
                    results.push(handle.await.unwrap());
                }
            }
        }

        results
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
