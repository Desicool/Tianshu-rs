//! Plan-and-Execute workflow.
//!
//! Implements a three-phase AI agent pattern:
//! 1. **Plan** — LLM generates a multi-step plan from the user query.
//! 2. **Execute** — Each plan step is run with tool access (checkpointed).
//! 3. **Synthesize** — LLM produces a final answer from all step results.
//!
//! All three phases are checkpoint-safe via `ctx.step()`. On restart, completed
//! steps are replayed from the store and the loop picks up where it left off.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tracing::info;

use example_tools::{run_tool_loop, ToolRegistry};
use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, LlmProvider, LlmRequest},
    workflow::{BaseWorkflow, WorkflowResult},
};

pub struct PlanAndExecuteWorkflow {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
}

#[async_trait]
impl BaseWorkflow for PlanAndExecuteWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        // Read the user's query from resource_data.
        let query = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["query"].as_str())
            .unwrap_or("(no query provided)")
            .to_string();

        info!(
            "PlanAndExecute: case_key={}, query={}",
            ctx.case.case_key, query
        );

        // ── Phase 1: Generate plan ────────────────────────────────────────────
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let query_for_plan = query.clone();

        let plan: Vec<String> = ctx
            .step("generate_plan", move |_ctx| async move {
                let request = LlmRequest {
                    model: model.clone(),
                    system_prompt: Some(
                        "You are a planning assistant. Break the user's task into 3-7 sequential \
                         steps. Respond with ONLY a JSON object: \
                         {\"steps\": [\"step 1\", \"step 2\", ...]}. No other text."
                            .to_string(),
                    ),
                    messages: vec![LlmMessage::user(query_for_plan.clone())],
                    temperature: None,
                    max_tokens: None,
                    tools: None,
                };

                let response = llm.complete(request).await?;
                let text = response.content.trim().to_string();

                // Parse JSON plan.
                let json: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| anyhow!("Plan response is not valid JSON: {e}. Got: {text}"))?;

                let steps: Vec<String> = json["steps"]
                    .as_array()
                    .ok_or_else(|| anyhow!("Plan JSON missing 'steps' array. Got: {text}"))?
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();

                info!("Generated plan with {} steps", steps.len());
                Ok(steps)
            })
            .await?;

        // ── Phase 2: Execute each plan step ───────────────────────────────────
        let mut prior_results: Vec<String> = Vec::with_capacity(plan.len());

        for (i, step_desc) in plan.iter().enumerate() {
            let llm = Arc::clone(&self.llm);
            let model = self.model.clone();
            let query_for_exec = query.clone();
            let plan_for_exec = plan.clone();
            let step_desc_for_exec = step_desc.clone();
            let prior_for_exec = prior_results.clone();
            let tools_arc = Arc::clone(&self.tools);

            let step_result: String = ctx
                .step(&format!("execute_step_{i}"), move |_ctx| async move {
                    // Build context message for the LLM.
                    let plan_text = plan_for_exec
                        .iter()
                        .enumerate()
                        .map(|(idx, s)| format!("  {}. {}", idx + 1, s))
                        .collect::<Vec<_>>()
                        .join("\n");

                    let prior_text = if prior_for_exec.is_empty() {
                        "None yet.".to_string()
                    } else {
                        prior_for_exec
                            .iter()
                            .enumerate()
                            .map(|(idx, r)| format!("  Step {}: {}", idx + 1, r))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };

                    let user_content = format!(
                        "Original task: {query_for_exec}\n\n\
                         Full plan:\n{plan_text}\n\n\
                         Results so far:\n{prior_text}\n\n\
                         Now execute this step: {step_desc_for_exec}"
                    );

                    let request = LlmRequest {
                        model: model.clone(),
                        system_prompt: Some(
                            "You are an execution assistant. Perform the requested step using \
                             available tools if needed. Return a concise summary of what you did \
                             and the result."
                                .to_string(),
                        ),
                        messages: vec![LlmMessage::user(user_content)],
                        temperature: None,
                        max_tokens: None,
                        tools: None, // run_tool_loop populates this from the registry
                    };

                    // run_tool_loop uses the registry both for tool definitions (sent to LLM)
                    // and for executing any tool calls the LLM returns.
                    let (result_text, _history) =
                        run_tool_loop(llm.as_ref(), request, &tools_arc, 5).await?;

                    info!("Step {i} result: {} chars", result_text.len());
                    Ok(result_text)
                })
                .await?;

            prior_results.push(step_result);
        }

        // ── Phase 3: Synthesize final answer ──────────────────────────────────
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let query_for_synth = query.clone();
        let results_for_synth = prior_results.clone();
        let plan_for_synth = plan.clone();

        let final_answer: String = ctx
            .step("synthesize", move |_ctx| async move {
                let step_summaries = plan_for_synth
                    .iter()
                    .zip(results_for_synth.iter())
                    .enumerate()
                    .map(|(i, (step, result))| {
                        format!("Step {}: {}\nResult: {}", i + 1, step, result)
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");

                let user_content = format!(
                    "Original task: {query_for_synth}\n\n\
                     Execution results:\n{step_summaries}\n\n\
                     Please synthesize a clear, comprehensive final answer."
                );

                let request = LlmRequest {
                    model: model.clone(),
                    system_prompt: Some(
                        "You are a synthesis assistant. Given a task and the results of executing \
                         a multi-step plan, provide a clear, comprehensive final answer."
                            .to_string(),
                    ),
                    messages: vec![LlmMessage::user(user_content)],
                    temperature: None,
                    max_tokens: None,
                    tools: None,
                };

                let response = llm.complete(request).await?;
                let answer = response.content.trim().to_string();
                info!("Synthesized final answer: {} chars", answer.len());
                Ok(answer)
            })
            .await?;

        ctx.finish("completed".to_string(), final_answer.clone())
            .await?;

        Ok(WorkflowResult::Finished(
            "completed".to_string(),
            final_answer,
        ))
    }
}
