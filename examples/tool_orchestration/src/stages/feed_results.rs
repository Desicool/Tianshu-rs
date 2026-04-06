//! FeedResults stage: loop back to PlanTools with cleared execution checkpoints.
//!
//! By clearing the previous execution-stage step keys, we ensure that
//! ExecuteConcurrent and ExecuteExclusive run fresh on the next loop iteration
//! (they won't read a cached checkpoint from the prior round).

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use workflow_engine::{
    context::WorkflowContext,
    stage::{StageBase, StageOutcome},
};

use crate::workflows::OrchStage;

pub struct FeedResultsStage;

#[async_trait]
impl StageBase<OrchStage> for FeedResultsStage {
    fn stage_key(&self) -> OrchStage {
        OrchStage::FeedResults
    }

    fn step_keys(&self) -> &[&str] {
        &["feed_results_step"]
    }

    async fn execute(&self, ctx: &mut WorkflowContext) -> Result<StageOutcome<OrchStage>> {
        info!(
            "FeedResults: looping back to PlanTools, case_key={}",
            ctx.case.case_key
        );

        // Clear the execution-stage checkpoints so they run fresh next iteration.
        Ok(StageOutcome::next_with_clear(
            OrchStage::PlanTools,
            vec![
                OrchStage::ExecuteConcurrent,
                OrchStage::ExecuteExclusive,
                OrchStage::FeedResults,
            ],
        ))
    }
}
