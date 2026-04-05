//! Tool orchestration workflow.
//!
//! Uses the **stage pattern** (`StageKey` + `StageBase` + `run_stages`) to
//! implement a multi-round LLM tool-use loop:
//!
//! ```text
//! PlanTools ──► ExecuteConcurrent ──► ExecuteExclusive ──► FeedResults ──┐
//!    ▲                                                                    │
//!    └──────────────────────── (loop until stop) ─────────────────────────┘
//!    │
//!    └──► Synthesize (when LLM returns "stop") ──► Finish
//! ```
//!
//! All stages checkpoint via `ctx.step()`, so the workflow is restart-safe.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use workflow_engine::{
    context::WorkflowContext,
    run_stages,
    stage::{StageBase, StageKey},
    workflow::{BaseWorkflow, WorkflowResult},
};

/// Stage key enum for the tool orchestration workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrchStage {
    PlanTools,
    ExecuteConcurrent,
    ExecuteExclusive,
    FeedResults,
    Synthesize,
}

impl StageKey for OrchStage {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PlanTools => "plan_tools",
            Self::ExecuteConcurrent => "execute_concurrent",
            Self::ExecuteExclusive => "execute_exclusive",
            Self::FeedResults => "feed_results",
            Self::Synthesize => "synthesize",
        }
    }
}

/// Parse an `OrchStage` from its string representation.
pub fn parse_stage(s: &str) -> Result<OrchStage> {
    match s {
        "plan_tools" => Ok(OrchStage::PlanTools),
        "execute_concurrent" => Ok(OrchStage::ExecuteConcurrent),
        "execute_exclusive" => Ok(OrchStage::ExecuteExclusive),
        "feed_results" => Ok(OrchStage::FeedResults),
        "synthesize" => Ok(OrchStage::Synthesize),
        other => Err(anyhow!("Unknown stage: {other}")),
    }
}

/// The tool orchestration workflow.
///
/// Drives the stage loop from the last-saved stage (or `start_stage` for new
/// cases).  Stages run to completion — the loop terminates when `Synthesize`
/// returns `StageOutcome::Finish`.
pub struct ToolOrchestrationWorkflow {
    pub stages: Arc<HashMap<OrchStage, Box<dyn StageBase<OrchStage>>>>,
    pub start_stage: OrchStage,
}

#[async_trait]
impl BaseWorkflow for ToolOrchestrationWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        // Resume from wherever we left off, defaulting to the start stage.
        let current_stage_str: String = ctx
            .get_state("current_stage", self.start_stage.as_str().to_string())
            .await?;
        let current_stage = parse_stage(&current_stage_str)?;
        run_stages(ctx, current_stage, &self.stages).await
    }
}
