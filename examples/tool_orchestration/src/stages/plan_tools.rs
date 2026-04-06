//! PlanTools stage: call the LLM and classify its response.
//!
//! - If the LLM returns `tool_use`: partition tool calls into concurrent/exclusive,
//!   save state, advance to `ExecuteConcurrent`.
//! - If the LLM returns `stop`: save the final assistant message, advance to
//!   `Synthesize`.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use example_tools::{ToolRegistry, ToolSafety};
use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, ToolCall},
    stage::{StageBase, StageOutcome},
};

use crate::workflows::OrchStage;

pub struct PlanToolsStage {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
}

#[async_trait]
impl StageBase<OrchStage> for PlanToolsStage {
    fn stage_key(&self) -> OrchStage {
        OrchStage::PlanTools
    }

    fn step_keys(&self) -> &[&str] {
        // Step keys are declared as a static pattern; the actual step names are
        // round-qualified at runtime (e.g. "plan_tools_step_0", "plan_tools_step_1").
        &["plan_tools_step"]
    }

    async fn execute(&self, ctx: &mut WorkflowContext) -> Result<StageOutcome<OrchStage>> {
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let tools_arc = Arc::clone(&self.tools);

        // Load or initialise the conversation history.
        let mut messages: Vec<LlmMessage> = ctx
            .get_state("orch_messages", Vec::<LlmMessage>::new())
            .await?;

        if messages.is_empty() {
            // First call — seed with the user's task from resource_data.
            let task = ctx
                .case
                .resource_data
                .as_ref()
                .and_then(|d| d["task"].as_str())
                .unwrap_or("(no task provided)")
                .to_string();
            messages.push(LlmMessage::user(task));
        }

        // Each call to PlanToolsStage gets a unique round number so that the
        // checkpoint key is distinct per round.  Without this, a multi-round
        // workflow would hit the round-0 cache on every subsequent round.
        let plan_round: u64 = ctx.get_state("orch_plan_round", 0u64).await?;
        let step_name = format!("plan_tools_step_{plan_round}");

        // Snapshot messages for the step closure (avoid borrowing ctx inside).
        let messages_snap = messages.clone();
        // Keep a second clone for use after the step (building updated_messages).
        let messages_before_step = messages_snap.clone();
        let llm_tools = tools_arc.to_llm_tools();

        // Checkpoint the LLM call so that crash+replay skips it and returns
        // the cached LlmResponse rather than calling the LLM again.
        let response: LlmResponse = ctx
            .step(&step_name, |_ctx| async move {
                let request = LlmRequest {
                    model: model.clone(),
                    system_prompt: Some(
                        "You are a helpful assistant with access to tools. \
                         Use tools when they are the most effective way to answer. \
                         When you have all the information you need, give a final answer directly."
                            .to_string(),
                    ),
                    messages: messages_snap.clone(),
                    temperature: None,
                    max_tokens: None,
                    tools: Some(llm_tools),
                };

                info!(
                    "PlanTools: calling LLM with {} messages",
                    request.messages.len()
                );

                llm.complete(request).await
            })
            .await?;

        // Advance the round counter so the next PlanTools call gets a fresh step name.
        ctx.set_state("orch_plan_round", plan_round + 1).await?;

        if response.finish_reason == "tool_use" {
            let tool_calls = response.tool_calls.unwrap_or_default();

            // Partition tool calls by safety classification.
            let (concurrent_calls, exclusive_calls): (Vec<ToolCall>, Vec<ToolCall>) =
                tool_calls.iter().cloned().partition(|c| {
                    tools_arc
                        .get(&c.name)
                        .map(|t| t.safety() == ToolSafety::ConcurrentSafe)
                        .unwrap_or(false) // unknown tools treated as exclusive
                });

            info!(
                "PlanTools: tool_use — {} concurrent, {} exclusive",
                concurrent_calls.len(),
                exclusive_calls.len()
            );

            // Append assistant message with tool calls to history.
            let mut updated_messages = messages_before_step.clone();
            updated_messages.push(LlmMessage::assistant_with_tool_calls(
                response.content,
                tool_calls.clone(),
            ));

            // Save state.
            ctx.set_state("orch_messages", updated_messages).await?;
            ctx.set_state("orch_concurrent_calls", concurrent_calls)
                .await?;
            ctx.set_state("orch_exclusive_calls", exclusive_calls)
                .await?;
            ctx.set_state("orch_pending_original_calls", tool_calls)
                .await?;

            Ok(StageOutcome::next(OrchStage::ExecuteConcurrent))
        } else {
            // LLM returned a final answer ("stop" or "end_turn").
            info!("PlanTools: stop — advancing to Synthesize");

            let mut updated_messages = messages_before_step.clone();
            updated_messages.push(LlmMessage::assistant(response.content));
            ctx.set_state("orch_messages", updated_messages).await?;

            Ok(StageOutcome::next(OrchStage::Synthesize))
        }
    }
}
