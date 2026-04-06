//! Leader workflow: plans tasks, assigns them to session state, waits for
//! worker results, then synthesizes a final answer.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::info;

use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, LlmProvider, LlmRequest},
    workflow::{BaseWorkflow, WorkflowResult},
};

/// A single subtask produced by the leader during planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: String,
    pub description: String,
}

// ── LeaderWorkflow ────────────────────────────────────────────────────────────

pub struct LeaderWorkflow {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
}

#[async_trait]
impl BaseWorkflow for LeaderWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        let task: String = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["task"].as_str())
            .unwrap_or("No task specified")
            .to_string();

        info!(
            "LeaderWorkflow: case_key={}, task={}",
            ctx.case.case_key, task
        );

        // ── Step 1: Plan tasks ────────────────────────────────────────────────
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let task_for_plan = task.clone();

        let subtasks: Vec<SubTask> = ctx
            .step("plan_tasks", move |_ctx| async move {
                let request = LlmRequest {
                    model: model.clone(),
                    system_prompt: Some(
                        "You are a task planner. Break the given task into exactly 2 subtasks. \
                         Return ONLY a JSON array with objects having 'id' (string, e.g. 'w1', 'w2') \
                         and 'description' (string) fields. No markdown, no extra text."
                            .to_string(),
                    ),
                    messages: vec![LlmMessage::user(format!(
                        "Break this task into 2 subtasks: {}",
                        task_for_plan
                    ))],
                    temperature: None,
                    max_tokens: None,
                    tools: None,
                };

                let response = llm.complete(request).await?;
                let subtasks: Vec<SubTask> = serde_json::from_str(&response.content)
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to parse subtasks JSON '{}': {}",
                            response.content,
                            e
                        )
                    })?;
                Ok(subtasks)
            })
            .await?;

        info!("LeaderWorkflow: planned {} subtasks", subtasks.len());

        // ── Step 2: Assign tasks to session state ─────────────────────────────
        // The step returns the manifest; we write session state after the step.
        let subtasks_for_assign = subtasks.clone();
        let assigned: bool = ctx
            .step("assign_tasks", move |_ctx| async move { Ok(true) })
            .await?;
        // Only write session state when this step was actually executed (not replayed).
        // Because ctx.step is idempotent, write unconditionally — writes are idempotent too.
        let _ = assigned;
        let manifest: Vec<String> = subtasks.iter().map(|s| s.id.clone()).collect();
        for subtask in &subtasks_for_assign {
            ctx.set_session_state(
                &format!("worker_task_{}", subtask.id),
                subtask.description.clone(),
            )
            .await?;
        }
        ctx.set_session_state("worker_manifest", manifest.clone())
            .await?;

        // ── Signal workers to be spawned ──────────────────────────────────────
        let spawned: bool = ctx.get_session_state("workers_spawned", false).await?;
        if !spawned {
            ctx.set_session_state("workers_spawned", true).await?;
            info!("LeaderWorkflow: signalling workers to be spawned");
            return Ok(WorkflowResult::Continue);
        }

        // ── Check if all workers are done ─────────────────────────────────────
        let manifest: Vec<String> = ctx
            .get_session_state("worker_manifest", Vec::<String>::new())
            .await?;

        let mut all_done = true;
        let mut results: Vec<String> = Vec::new();

        for id in &manifest {
            let result: String = ctx
                .get_session_state(&format!("worker_result_{}", id), String::new())
                .await?;
            if result.is_empty() {
                all_done = false;
                break;
            }
            results.push(format!("Worker {}: {}", id, result));
        }

        if !all_done {
            info!("LeaderWorkflow: workers not done yet, continuing to wait");
            return Ok(WorkflowResult::Continue);
        }

        info!("LeaderWorkflow: all workers done, synthesizing final answer");

        // ── Step 3: Synthesize final answer ───────────────────────────────────
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let results_for_synth = results.clone();
        let original_task = task.clone();

        let final_answer: String = ctx
            .step("synthesize", move |_ctx| async move {
                let combined = results_for_synth.join("\n");
                let request = LlmRequest {
                    model: model.clone(),
                    system_prompt: Some(
                        "You are a synthesizer. Given a task and worker results, \
                         produce a concise final answer."
                            .to_string(),
                    ),
                    messages: vec![LlmMessage::user(format!(
                        "Original task: {}\n\nWorker results:\n{}",
                        original_task, combined
                    ))],
                    temperature: None,
                    max_tokens: None,
                    tools: None,
                };

                let response = llm.complete(request).await?;
                Ok(response.content)
            })
            .await?;

        let finished_answer = final_answer.clone();
        ctx.finish("completed".to_string(), final_answer).await?;
        Ok(WorkflowResult::Finished(
            "completed".to_string(),
            finished_answer,
        ))
    }
}
