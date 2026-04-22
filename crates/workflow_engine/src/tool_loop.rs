// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use chrono::Utc;

use crate::agent::AgentId;
use crate::llm::{LlmMessage, LlmProvider, LlmRequest};
use crate::observe::{Observer, ToolCallRecord};
use crate::tool::{ScopedToolRegistry, ToolRegistry};

/// Configuration for the tool-use loop.
pub struct ToolLoopConfig {
    pub max_rounds: usize,
    pub max_concurrency: usize,
}

impl Default for ToolLoopConfig {
    fn default() -> Self {
        Self {
            max_rounds: 10,
            max_concurrency: 5,
        }
    }
}

/// Result of a completed tool loop.
#[derive(Debug)]
pub struct ToolLoopResult {
    pub final_text: String,
    pub messages: Vec<LlmMessage>,
    pub rounds: usize,
    pub total_tool_calls: usize,
}

/// Run the LLM tool-use loop.
///
/// Repeatedly calls the LLM. When the LLM returns tool calls, they are
/// executed via `tools` and the results are appended to the conversation.
/// The loop terminates when the LLM returns a text response (no tool calls)
/// or `config.max_rounds` is exceeded.
pub async fn run_tool_loop(
    llm: &dyn LlmProvider,
    request: LlmRequest,
    tools: &ToolRegistry,
    config: &ToolLoopConfig,
    observer: Option<&dyn Observer>,
) -> Result<ToolLoopResult> {
    let mut messages = request.messages.clone();
    let mut rounds = 0;
    let mut total_tool_calls = 0;

    loop {
        if rounds >= config.max_rounds {
            return Err(anyhow::anyhow!(
                "tool loop exceeded max_rounds={}",
                config.max_rounds
            ));
        }

        let req = LlmRequest {
            messages: messages.clone(),
            tools: Some(tools.to_llm_tools()),
            model: request.model.clone(),
            system_prompt: request.system_prompt.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let response = llm.complete(req).await?;
        rounds += 1;

        // Add assistant message to history
        messages.push(LlmMessage {
            role: "assistant".into(),
            content: response.content.clone(),
            tool_calls: response.tool_calls.clone(),
            tool_call_id: None,
        });

        let calls = response.tool_calls.unwrap_or_default();
        if calls.is_empty() {
            return Ok(ToolLoopResult {
                final_text: response.content,
                messages,
                rounds,
                total_tool_calls,
            });
        }

        total_tool_calls += calls.len();

        // Execute tool calls with concurrency; each entry carries its own duration.
        let results = tools
            .execute_with_concurrency(&calls, config.max_concurrency)
            .await;

        // Emit observer events using per-tool durations
        if let Some(obs) = observer {
            for (idx, (result, duration_ms)) in results.iter().enumerate() {
                let call = &calls[idx];
                let input: serde_json::Value =
                    serde_json::from_str(&call.arguments).unwrap_or_default();

                let record = ToolCallRecord {
                    case_key: String::new(),
                    step_name: None,
                    tool_name: call.name.clone(),
                    call_id: call.id.clone(),
                    input,
                    output: Some(result.content.clone()),
                    is_error: result.is_error,
                    duration_ms: *duration_ms,
                    timestamp: Utc::now(),
                    agent_id: None,
                };
                obs.on_tool_call(&record).await;
            }
        }

        // Add tool result messages
        for (result, _duration_ms) in results {
            messages.push(LlmMessage {
                role: "tool".into(),
                content: result.content.clone(),
                tool_calls: None,
                tool_call_id: Some(result.call_id.clone()),
            });
        }
    }
}

/// Like `run_tool_loop` but uses a capability-scoped tool registry and tags
/// observer records with the agent's ID.
pub async fn run_agent_tool_loop(
    llm: &dyn LlmProvider,
    request: LlmRequest,
    tools: &ScopedToolRegistry<'_>,
    config: &ToolLoopConfig,
    observer: Option<&dyn Observer>,
    agent_id: &AgentId,
) -> Result<ToolLoopResult> {
    let mut messages = request.messages.clone();
    let mut rounds = 0;
    let mut total_tool_calls = 0;

    loop {
        if rounds >= config.max_rounds {
            return Err(anyhow::anyhow!(
                "tool loop exceeded max_rounds={}",
                config.max_rounds
            ));
        }

        let req = LlmRequest {
            messages: messages.clone(),
            tools: Some(tools.to_llm_tools()),
            model: request.model.clone(),
            system_prompt: request.system_prompt.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let response = llm.complete(req).await?;
        rounds += 1;

        // Add assistant message to history
        messages.push(LlmMessage {
            role: "assistant".into(),
            content: response.content.clone(),
            tool_calls: response.tool_calls.clone(),
            tool_call_id: None,
        });

        let calls = response.tool_calls.unwrap_or_default();
        if calls.is_empty() {
            return Ok(ToolLoopResult {
                final_text: response.content,
                messages,
                rounds,
                total_tool_calls,
            });
        }

        total_tool_calls += calls.len();

        // Execute tool calls with concurrency; each entry carries its own duration.
        let results = tools
            .execute_with_concurrency(&calls, config.max_concurrency)
            .await;

        // Emit observer events using per-tool durations, tagged with agent_id
        if let Some(obs) = observer {
            for (idx, (result, duration_ms)) in results.iter().enumerate() {
                let call = &calls[idx];
                let input: serde_json::Value =
                    serde_json::from_str(&call.arguments).unwrap_or_default();

                let record = ToolCallRecord {
                    case_key: String::new(),
                    step_name: None,
                    tool_name: call.name.clone(),
                    call_id: call.id.clone(),
                    input,
                    output: Some(result.content.clone()),
                    is_error: result.is_error,
                    duration_ms: *duration_ms,
                    timestamp: Utc::now(),
                    agent_id: Some(agent_id.clone()),
                };
                obs.on_tool_call(&record).await;
            }
        }

        // Add tool result messages
        for (result, _duration_ms) in results {
            messages.push(LlmMessage {
                role: "tool".into(),
                content: result.content.clone(),
                tool_calls: None,
                tool_call_id: Some(result.call_id.clone()),
            });
        }
    }
}
