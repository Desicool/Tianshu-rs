//! ExecuteExclusive stage: run Exclusive tool calls sequentially, then merge
//! all results back into the conversation history in original call order.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use example_tools::{ToolRegistry, ToolSafety};
use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, ToolCall, ToolResult},
    stage::{StageBase, StageOutcome},
};

use crate::workflows::OrchStage;

pub struct ExecuteExclusiveStage {
    pub tools: Arc<ToolRegistry>,
}

#[async_trait]
impl StageBase<OrchStage> for ExecuteExclusiveStage {
    fn stage_key(&self) -> OrchStage {
        OrchStage::ExecuteExclusive
    }

    fn step_keys(&self) -> &[&str] {
        &["execute_exclusive_step"]
    }

    async fn execute(&self, ctx: &mut WorkflowContext) -> Result<StageOutcome<OrchStage>> {
        let exclusive_calls: Vec<ToolCall> = ctx
            .get_state("orch_exclusive_calls", Vec::<ToolCall>::new())
            .await?;

        // Execute exclusive calls sequentially.
        let mut exclusive_results: Vec<ToolResult> = Vec::with_capacity(exclusive_calls.len());
        for call in &exclusive_calls {
            info!("ExecuteExclusive: running '{}'", call.name);
            let result = self.tools.execute_call(call).await;
            exclusive_results.push(result);
        }

        // Load concurrent results and original call order.
        let concurrent_results: Vec<ToolResult> = ctx
            .get_state("orch_concurrent_results", Vec::<ToolResult>::new())
            .await?;

        let original_calls: Vec<ToolCall> = ctx
            .get_state("orch_pending_original_calls", Vec::<ToolCall>::new())
            .await?;

        // Merge results in the original tool_calls order.
        // Walk the original call list and assign results from the appropriate bucket.
        let mut safe_iter = concurrent_results.into_iter();
        let mut excl_iter = exclusive_results.into_iter();
        let mut merged: Vec<ToolResult> = Vec::with_capacity(original_calls.len());

        for call in &original_calls {
            let safety = self
                .tools
                .get(&call.name)
                .map(|t| t.safety())
                .unwrap_or(ToolSafety::Exclusive);

            let result = if safety == ToolSafety::ConcurrentSafe {
                safe_iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("concurrent result missing for '{}'", call.name))?
            } else {
                excl_iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("exclusive result missing for '{}'", call.name))?
            };
            merged.push(result);
        }

        info!("ExecuteExclusive: merged {} total results", merged.len());

        // Append the tool results message to the conversation history.
        let mut messages: Vec<LlmMessage> = ctx
            .get_state("orch_messages", Vec::<LlmMessage>::new())
            .await?;
        messages.push(LlmMessage::tool_results(merged));
        ctx.set_state("orch_messages", messages).await?;

        // Clear the pending call state — it will be repopulated in the next PlanTools run.
        ctx.set_state("orch_concurrent_calls", Vec::<ToolCall>::new())
            .await?;
        ctx.set_state("orch_exclusive_calls", Vec::<ToolCall>::new())
            .await?;
        ctx.set_state("orch_pending_original_calls", Vec::<ToolCall>::new())
            .await?;
        ctx.set_state("orch_concurrent_results", Vec::<ToolResult>::new())
            .await?;

        Ok(StageOutcome::next(OrchStage::FeedResults))
    }
}
