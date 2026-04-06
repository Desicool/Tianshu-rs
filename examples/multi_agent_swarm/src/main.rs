//! Multi-agent swarm demo.
//!
//! Demonstrates the leader/worker pattern:
//! 1. Leader plans tasks and assigns them to session state.
//! 2. Workers are spawned and execute their tasks.
//! 3. Leader waits for all worker results, then synthesizes a final answer.
//!
//! Requires `OPENAI_API_KEY` (or compatible) environment variable.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tracing::info;

use workflow_engine::{
    case::Case,
    engine::{ExecutionMode, SchedulerEnvironment, SchedulerV2},
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
    ExecutionState, WorkflowRegistry,
};
use workflow_engine_llm_openai::OpenAiProvider;

use multi_agent_swarm::register_workflows;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-dummy".to_string());
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let base_url = std::env::var("OPENAI_BASE_URL").ok();
    let llm = Arc::new(match base_url {
        Some(url) => OpenAiProvider::builder(api_key, model.clone())
            .base_url(url)
            .build(),
        None => OpenAiProvider::new(api_key, model.clone()),
    });

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm, model);

    let session_id = "swarm_demo";
    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());

    let mut leader_case = Case::new("leader".into(), session_id.into(), "swarm_leader".into());
    leader_case.resource_data = Some(json!({
        "task": "Research the benefits and drawbacks of microservices architecture"
    }));

    let mut env = SchedulerEnvironment::from_session_id(session_id, vec![leader_case])
        .with_execution_mode(ExecutionMode::Parallel);

    let mut sched = SchedulerV2::new();
    let mut workers_added = false;

    info!("Starting multi-agent swarm demo...");

    loop {
        sched
            .tick(
                &mut env,
                &registry,
                Arc::clone(&cs) as Arc<dyn CaseStore>,
                Arc::clone(&ss) as Arc<dyn StateStore>,
                None,
                None,
            )
            .await?;

        // Check if we need to spawn workers.
        if !workers_added {
            let spawned_entry = ss
                .get_session(session_id, "wf_sess_state_workers_spawned")
                .await?;
            let spawned: bool = spawned_entry
                .map(|e| serde_json::from_str(&e.data).unwrap_or(false))
                .unwrap_or(false);

            if spawned {
                let manifest_entry = ss
                    .get_session(session_id, "wf_sess_state_worker_manifest")
                    .await?;

                if let Some(entry) = manifest_entry {
                    let manifest: Vec<String> = serde_json::from_str(&entry.data)?;
                    info!("Spawning {} workers: {:?}", manifest.len(), manifest);
                    for id in &manifest {
                        let mut worker = Case::new(
                            format!("worker_{}", id),
                            session_id.into(),
                            "swarm_worker".into(),
                        );
                        worker.resource_data = Some(json!({ "worker_id": id }));
                        env.current_case_dict
                            .insert(worker.case_key.clone(), worker);
                    }
                    workers_added = true;
                }
            }
        }

        // Check if leader is finished.
        if let Some(leader) = env.current_case_dict.get("leader") {
            if leader.execution_state == ExecutionState::Finished {
                info!(
                    "Leader finished! type={:?}, description={:?}",
                    leader.finished_type, leader.finished_description
                );
                println!(
                    "\n=== Final Answer ===\n{}",
                    leader
                        .finished_description
                        .as_deref()
                        .unwrap_or("(no answer)")
                );
                break;
            }
        }

        // Check if all cases are finished or no active cases remain.
        let all_done = env
            .current_case_dict
            .values()
            .all(|c| c.execution_state == ExecutionState::Finished);
        if all_done {
            info!("All cases finished.");
            break;
        }
    }

    Ok(())
}
