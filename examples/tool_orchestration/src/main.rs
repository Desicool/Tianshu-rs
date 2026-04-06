//! Tool Orchestration workflow CLI runner.
//!
//! Reads a task from CLI args (joined with spaces) or uses a built-in demo task.
//!
//! Environment variables:
//!   OPENAI_API_KEY   — required
//!   OPENAI_MODEL     — model name (default: "gpt-4o-mini")
//!   OPENAI_BASE_URL  — API base URL (default: OpenAI)
//!
//! Usage:
//!   cargo run -p tool_orchestration -- "List the files in /tmp"
use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use workflow_engine::{
    case::Case,
    engine::{shutdown_signal, SchedulerEnvironment, SchedulerV2},
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
    WorkflowRegistry,
};
use workflow_engine_llm_openai::OpenAiProvider;
use workflow_engine_observe::{
    dataset::{llm_dataset, step_dataset, workflow_dataset},
    InMemoryObserver,
};

use tool_orchestration::register_workflows;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // ── Configuration from environment ───────────────────────────────────────
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-placeholder".to_string());
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let base_url = std::env::var("OPENAI_BASE_URL").ok();

    // ── Task from CLI args or default ─────────────────────────────────────────
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = if args.is_empty() {
        "List the files in /tmp and report how many there are.".to_string()
    } else {
        args.join(" ")
    };

    info!("Task: {}", task);
    info!("Model: {}", model);

    // ── LLM provider ─────────────────────────────────────────────────────────
    let llm: Arc<dyn workflow_engine::llm::LlmProvider> = if let Some(url) = base_url {
        Arc::new(
            OpenAiProvider::builder(&api_key, &model)
                .base_url(url)
                .build(),
        )
    } else {
        Arc::new(OpenAiProvider::new(&api_key, &model))
    };

    // ── Stores and registry ───────────────────────────────────────────────────
    let case_store: Arc<dyn CaseStore> = Arc::new(InMemoryCaseStore::default());
    let state_store: Arc<dyn StateStore> = Arc::new(InMemoryStateStore::default());

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, Arc::clone(&llm), model.clone());

    // ── Case setup ────────────────────────────────────────────────────────────
    let case_key = "tool_orchestration_demo_001";
    let mut case = Case::new(
        case_key.into(),
        "demo_session".into(),
        "tool_orchestration".into(),
    );
    case.resource_data = Some(serde_json::json!({ "task": task }));

    // ── Observability ─────────────────────────────────────────────────────────
    let memory_obs = Arc::new(InMemoryObserver::new());
    let signal = shutdown_signal();

    let mut env = SchedulerEnvironment::from_session_id("demo_session", vec![case]);
    let mut scheduler = SchedulerV2::new();
    scheduler.set_observer(memory_obs.clone());

    // ── Tick loop until finished ──────────────────────────────────────────────
    info!("Starting tool-orchestration workflow...");

    loop {
        let result = scheduler
            .tick(
                &mut env,
                &registry,
                Arc::clone(&case_store),
                Arc::clone(&state_store),
                None,
                Some(&signal),
            )
            .await?;

        use workflow_engine::engine::TickResult;
        match result {
            TickResult::ShutdownRequested => {
                info!("Shutdown requested, exiting.");
                break;
            }
            TickResult::Idle => {
                info!("No runnable workflows.");
                break;
            }
            TickResult::Executed => {}
        }

        let case = &env.current_case_dict[case_key];
        use workflow_engine::case::ExecutionState;
        if case.execution_state == ExecutionState::Finished {
            break;
        }
    }

    // ── Print result ──────────────────────────────────────────────────────────
    let final_case = &env.current_case_dict[case_key];
    info!(
        "Workflow complete: state={:?}, type={:?}",
        final_case.execution_state, final_case.finished_type
    );

    if let Some(desc) = &final_case.finished_description {
        println!("\n=== Final Answer ===\n{desc}\n");
    }

    // ── Observability summary ─────────────────────────────────────────────────
    let step_records = memory_obs.step_records();
    let workflow_records = memory_obs.workflow_records();
    let llm_records = memory_obs.llm_records();

    info!("--- Observability Summary ---");
    info!("  Steps recorded:     {}", step_records.len());
    info!("  Workflows finished: {}", workflow_records.len());
    info!("  LLM calls recorded: {}", llm_records.len());

    for sr in &step_records {
        info!(
            "  step '{}': cached={}, duration={}ms",
            sr.step_name, sr.cached, sr.duration_ms
        );
    }

    let step_entries = step_dataset(&step_records);
    let wf_entries = workflow_dataset(&workflow_records);
    let llm_entries = llm_dataset(&llm_records);

    info!("  RLHF-ready step entries: {}", step_entries.len());
    info!("  RLHF-ready workflow entries: {}", wf_entries.len());
    info!("  RLHF-ready LLM call entries: {}", llm_entries.len());

    Ok(())
}
