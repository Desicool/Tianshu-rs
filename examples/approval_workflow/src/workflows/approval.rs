//! Approval workflow: submit document → wait for review decision → approved/rejected.
//!
//! # Design
//!
//! The workflow runs in three logical phases:
//!
//! 1. **Submit** — checkpoint the document submission (idempotent via `ctx.step`)
//! 2. **Wait for review** — emit a poll predicate and return `Waiting`. The
//!    orchestrator loop is responsible for fetching the review decision and
//!    writing it into `case.resource_data` before the next tick.
//! 3. **Decide** — read the decision from `resource_data` and finish.
//!
//! # Resource data convention
//!
//! Input (set by caller before creating the case):
//! ```json
//! { "document_id": "...", "submitter": "...", "title": "..." }
//! ```
//!
//! After review decision arrives (set by orchestrator before tick 2):
//! ```json
//! { "decision": "approved" | "rejected", "reviewer": "...", "reason": "..." }
//! ```
use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use tianshu::{
    context::WorkflowContext,
    workflow::{BaseWorkflow, PollPredicate, WorkflowResult},
};

pub struct ApprovalWorkflow;

#[async_trait]
impl BaseWorkflow for ApprovalWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        // ── Phase 1: Submit (idempotent) ──────────────────────────────────────
        let doc_title = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["title"].as_str())
            .unwrap_or("(untitled)")
            .to_string();
        let case_key = ctx.case.case_key.clone();

        ctx.step("submitted", move |_ctx| {
            let title = doc_title.clone();
            let key = case_key.clone();
            async move {
                info!("Document submitted: case_key={}, title={}", key, title);
                Ok(title)
            }
        })
        .await?;

        // ── Phase 2: Check for review decision ────────────────────────────────
        // The orchestrator writes the decision into resource_data before the
        // next tick, following the same pattern as the business code.
        let decision = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["decision"].as_str().map(String::from));

        if decision.is_none() {
            info!(
                "Waiting for review decision: case_key={}",
                ctx.case.case_key
            );
            return Ok(WorkflowResult::Waiting(vec![PollPredicate {
                resource_type: "review_decision".into(),
                resource_id: ctx.case.case_key.clone(),
                step_name: "review_decision_received".into(),
                intent_desc: None,
            }]));
        }

        // ── Phase 3: Process decision ─────────────────────────────────────────
        let decision = decision.unwrap();
        let reason = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["reason"].as_str())
            .unwrap_or(&decision)
            .to_string();

        info!(
            "Processing decision: case_key={}, decision={}",
            ctx.case.case_key, decision
        );

        ctx.finish(decision.clone(), reason.clone()).await?;
        Ok(WorkflowResult::Finished(decision, reason))
    }
}
