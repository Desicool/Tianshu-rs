//! Worker workflow: reads a task from session state, executes it with LLM +
//! tools, then reports the result back to session state.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use example_tools::{run_tool_loop, ToolRegistry};
use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, LlmProvider, LlmRequest},
    workflow::{BaseWorkflow, WorkflowResult},
};

// ── WorkerWorkflow ────────────────────────────────────────────────────────────

pub struct WorkerWorkflow {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
}

#[async_trait]
impl BaseWorkflow for WorkerWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        let worker_id: String = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["worker_id"].as_str())
            .unwrap_or("unknown")
            .to_string();

        info!(
            "WorkerWorkflow: case_key={}, worker_id={}",
            ctx.case.case_key, worker_id
        );

        // ── Step 1: Read task from session state ──────────────────────────────
        // Read directly from session state (outside ctx.step, since we just need
        // to look up shared state — no expensive computation to checkpoint here).
        let task_desc: String = ctx
            .get_session_state(&format!("worker_task_{}", worker_id), String::new())
            .await?;

        info!(
            "WorkerWorkflow: worker_id={}, task={}",
            worker_id, task_desc
        );

        // ── Step 2: Execute task with LLM + tools ─────────────────────────────
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let tools = Arc::clone(&self.tools);
        let task_for_exec = task_desc.clone();
        let wid_for_exec = worker_id.clone();

        let result: String = ctx
            .step("execute_task", move |_ctx| async move {
                let request = LlmRequest {
                    model: model.clone(),
                    system_prompt: Some(format!(
                        "You are worker agent {}. Complete the given task concisely.",
                        wid_for_exec
                    )),
                    messages: vec![LlmMessage::user(task_for_exec)],
                    temperature: None,
                    max_tokens: None,
                    tools: Some(tools.to_llm_tools()),
                };

                let (answer, _messages) = run_tool_loop(llm.as_ref(), request, &tools, 3).await?;
                Ok(answer)
            })
            .await?;

        info!(
            "WorkerWorkflow: worker_id={}, result_len={}",
            worker_id,
            result.len()
        );

        // ── Step 3: Report result to session state ────────────────────────────
        // Use step for idempotency, write session state after the step checkpoint.
        let _reported: bool = ctx
            .step("report_result", move |_ctx| async move { Ok(true) })
            .await?;
        ctx.set_session_state(&format!("worker_result_{}", worker_id), result.clone())
            .await?;

        let result_clone = result.clone();
        ctx.finish("completed".to_string(), result).await?;
        Ok(WorkflowResult::Finished(
            "completed".to_string(),
            result_clone,
        ))
    }
}
