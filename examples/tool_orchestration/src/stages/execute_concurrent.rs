//! ExecuteConcurrent stage: run all ConcurrentSafe tool calls in parallel.
//!
//! If there are no concurrent calls, the stage advances immediately with an
//! empty results list.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use example_tools::ToolRegistry;
use workflow_engine::{
    context::WorkflowContext,
    llm::{ToolCall, ToolResult},
    stage::{StageBase, StageOutcome},
};

use crate::workflows::OrchStage;

pub struct ExecuteConcurrentStage {
    pub tools: Arc<ToolRegistry>,
}

#[async_trait]
impl StageBase<OrchStage> for ExecuteConcurrentStage {
    fn stage_key(&self) -> OrchStage {
        OrchStage::ExecuteConcurrent
    }

    fn step_keys(&self) -> &[&str] {
        &["execute_concurrent_step"]
    }

    async fn execute(&self, ctx: &mut WorkflowContext) -> Result<StageOutcome<OrchStage>> {
        let calls: Vec<ToolCall> = ctx
            .get_state("orch_concurrent_calls", Vec::<ToolCall>::new())
            .await?;

        if calls.is_empty() {
            info!("ExecuteConcurrent: no concurrent calls, skipping");
            ctx.set_state("orch_concurrent_results", Vec::<ToolResult>::new())
                .await?;
            return Ok(StageOutcome::next(OrchStage::ExecuteExclusive));
        }

        info!(
            "ExecuteConcurrent: executing {} calls in parallel",
            calls.len()
        );

        let results = self.tools.execute_calls_parallel(&calls).await;

        info!("ExecuteConcurrent: got {} results", results.len());

        ctx.set_state("orch_concurrent_results", results).await?;

        Ok(StageOutcome::next(OrchStage::ExecuteExclusive))
    }
}
