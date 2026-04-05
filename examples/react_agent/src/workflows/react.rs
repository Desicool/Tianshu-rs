//! ReAct (Reason-Act-Observe) agent workflow.
//!
//! Each tick of the workflow performs ONE LLM call:
//!
//! * If the model requests tools, the tools are executed (ConcurrentSafe ones
//!   in parallel, Exclusive ones sequentially) and the results are appended to
//!   the accumulated message history.  The engine then re-ticks immediately via
//!   `WorkflowResult::Continue`.
//!
//! * If the model returns a final answer (finish_reason != "tool_use") the
//!   workflow finishes.
//!
//! Checkpointing: each tick is wrapped in `ctx.step("react_iter_{n}")`, so a
//! crash mid-iteration is safe — completed iterations are replayed from the
//! store and the loop resumes from the next unfinished iteration.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::info;

use example_tools::{ToolRegistry, ToolSafety};
use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, LlmProvider, LlmRequest, ToolCall, ToolResult},
    workflow::{BaseWorkflow, WorkflowResult},
};

// ── ReActOutcome ──────────────────────────────────────────────────────────────

/// The outcome of a single ReAct iteration step.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum ReActOutcome {
    /// LLM called tools; updated message history is stored for the next tick.
    Continue { messages: Vec<LlmMessage> },
    /// LLM produced a final answer — no more tool calls.
    Done { answer: String },
}

// ── ReactAgentWorkflow ────────────────────────────────────────────────────────

pub struct ReactAgentWorkflow {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
    /// Maximum number of reasoning iterations before giving up.
    pub max_iterations: usize,
}

impl Default for ReactAgentWorkflow {
    fn default() -> Self {
        panic!("ReactAgentWorkflow requires llm, model, and tools");
    }
}

#[async_trait]
impl BaseWorkflow for ReactAgentWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        // Read the user's task from resource_data.
        let task: String = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["task"].as_str())
            .unwrap_or("No task specified")
            .to_string();

        let iteration: usize = ctx.get_state("react_iteration", 0usize).await?;

        info!(
            "ReactAgent: case_key={}, iteration={}, task={}",
            ctx.case.case_key, iteration, task
        );

        // Guard against infinite loops.
        if iteration >= self.max_iterations {
            let msg = format!("Exceeded maximum iterations ({})", self.max_iterations);
            ctx.finish("max_iterations_reached".to_string(), msg.clone())
                .await?;
            return Ok(WorkflowResult::Finished(
                "max_iterations_reached".to_string(),
                msg,
            ));
        }

        // ── Capture values that must be moved into the step closure ───────────
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let tools = Arc::clone(&self.tools);
        let task_for_step = task.clone();

        // Load message history (or build initial message on first iteration).
        let messages_value: serde_json::Value = ctx
            .get_state("react_messages", serde_json::Value::Null)
            .await?;

        let initial_messages: Vec<LlmMessage> = if messages_value.is_null() {
            vec![LlmMessage::user(task_for_step.clone())]
        } else {
            serde_json::from_value(messages_value)?
        };

        // ── One checkpointed iteration ────────────────────────────────────────
        let outcome: ReActOutcome = ctx
            .step(&format!("react_iter_{iteration}"), move |_ctx| async move {
                // Build the LLM request with current message history.
                let request = LlmRequest {
                    model: model.clone(),
                    system_prompt: Some(
                        "You are a ReAct agent that solves tasks step by step using available \
                         tools.\n\nThink step by step. On each turn:\n\
                         - You may call one or more tools to gather information\n\
                         - Or provide your final answer if you have enough information\n\n\
                         Be concise and systematic. Use tools when needed."
                            .to_string(),
                    ),
                    messages: initial_messages.clone(),
                    temperature: None,
                    max_tokens: None,
                    tools: Some(tools.to_llm_tools()),
                };

                // One LLM call — the heart of the ReAct "Thought" step.
                let response = llm.complete(request).await?;

                if response.finish_reason != "tool_use" {
                    // Final answer — no more tools needed.
                    return Ok(ReActOutcome::Done {
                        answer: response.content,
                    });
                }

                // ── Execute tool calls ────────────────────────────────────────
                let tool_calls = response.tool_calls.unwrap_or_default();
                let results = execute_tools_partitioned(tools.as_ref(), &tool_calls).await;

                // Build updated message history.
                let mut new_messages = initial_messages;
                new_messages.push(LlmMessage::assistant_with_tool_calls(
                    response.content,
                    tool_calls,
                ));
                new_messages.push(LlmMessage::tool_results(results));

                Ok(ReActOutcome::Continue {
                    messages: new_messages,
                })
            })
            .await?;

        // ── Handle the outcome ────────────────────────────────────────────────
        match outcome {
            ReActOutcome::Done { answer } => {
                info!(
                    "ReactAgent finished: case_key={}, answer_len={}",
                    ctx.case.case_key,
                    answer.len()
                );
                ctx.finish("completed".to_string(), answer.clone()).await?;
                Ok(WorkflowResult::Finished("completed".to_string(), answer))
            }
            ReActOutcome::Continue { messages } => {
                // Persist updated history and bump the iteration counter.
                ctx.set_state("react_messages", serde_json::to_value(&messages)?)
                    .await?;
                ctx.set_state("react_iteration", iteration + 1).await?;
                info!(
                    "ReactAgent continuing: case_key={}, next_iteration={}",
                    ctx.case.case_key,
                    iteration + 1
                );
                Ok(WorkflowResult::Continue)
            }
        }
    }
}

// ── Tool execution helper ─────────────────────────────────────────────────────

/// Execute `calls` with safety-aware parallelism, returning results in the
/// original call order.
async fn execute_tools_partitioned(registry: &ToolRegistry, calls: &[ToolCall]) -> Vec<ToolResult> {
    // Partition by safety.
    let (safe_calls, exclusive_calls): (Vec<_>, Vec<_>) = calls.iter().cloned().partition(|c| {
        registry
            .get(&c.name)
            .map(|t| t.safety() == ToolSafety::ConcurrentSafe)
            .unwrap_or(false) // unknown tools treated as exclusive
    });

    // Run safe calls concurrently.
    let safe_results = registry.execute_calls_parallel(&safe_calls).await;

    // Run exclusive calls sequentially.
    let mut exclusive_results = Vec::with_capacity(exclusive_calls.len());
    for call in &exclusive_calls {
        exclusive_results.push(registry.execute_call(call).await);
    }

    // Merge back in original order.
    let mut safe_iter = safe_results.into_iter();
    let mut excl_iter = exclusive_results.into_iter();
    let mut results = Vec::with_capacity(calls.len());
    for call in calls {
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
    results
}
