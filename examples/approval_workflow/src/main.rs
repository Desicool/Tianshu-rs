//! Approval workflow orchestrator.
//!
//! Demonstrates running the workflow engine with in-memory stores and observability.
//! Pass `--postgres <DATABASE_URL>` to use PostgreSQL instead.
//! Pass `--jsonl <PATH>` to write traces to a JSONL file.
//! Pass `--parallel` to run multiple workflows in parallel mode.
//!
//! Usage:
//!   cargo run -p approval_workflow
//!   cargo run -p approval_workflow -- --jsonl /tmp/traces.jsonl
//!   cargo run -p approval_workflow -- --postgres postgres://user:pass@localhost/db
//!   cargo run -p approval_workflow -- --parallel
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::info;

use workflow_engine::{
    case::Case,
    engine::{shutdown_signal, ExecutionMode, SchedulerEnvironment, SchedulerV2, TickResult},
    poll::ResourceFetcher,
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
    WorkflowRegistry,
};
use workflow_engine_observe::{
    dataset::{llm_dataset, step_dataset, workflow_dataset},
    InMemoryObserver, JsonlObserver, CompositeObserver,
};

use approval_workflow::register_workflows;

// ── Simple in-process event bus ───────────────────────────────────────────────
// In a real application this would poll a database or message queue.

use std::collections::HashMap;
use std::sync::RwLock;
use serde_json::Value as JsonValue;
use async_trait::async_trait;

#[derive(Default)]
struct EventBus {
    events: RwLock<HashMap<String, JsonValue>>,
}

impl EventBus {
    fn publish(&self, case_key: &str, payload: JsonValue) {
        self.events.write().unwrap().insert(case_key.to_string(), payload);
    }
}

#[async_trait]
impl ResourceFetcher for EventBus {
    async fn fetch(&self, resource_type: &str, resource_id: &str) -> anyhow::Result<Option<JsonValue>> {
        if resource_type == "review_decision" {
            Ok(self.events.read().unwrap().get(resource_id).cloned())
        } else {
            Ok(None)
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let args: Vec<String> = std::env::args().collect();
    let use_postgres = args.windows(2).any(|w| w[0] == "--postgres");
    let use_parallel = args.iter().any(|a| a == "--parallel");

    let (case_store, state_store): (Arc<dyn CaseStore>, Arc<dyn StateStore>) = if use_postgres {
        #[cfg(feature = "postgres")]
        {
            let db_url = args
                .windows(2)
                .find(|w| w[0] == "--postgres")
                .map(|w| w[1].clone())
                .expect("--postgres requires a DATABASE_URL argument");
            let pool = workflow_engine_postgres::build_pool(&db_url)?;
            let cs = Arc::new(workflow_engine_postgres::PostgresCaseStore::new(pool.clone()));
            let ss = Arc::new(workflow_engine_postgres::PostgresStateStore::new(pool));
            cs.setup().await?;
            ss.setup().await?;
            (cs, ss)
        }
        #[cfg(not(feature = "postgres"))]
        {
            eprintln!("Postgres feature not enabled. Using in-memory stores.");
            (
                Arc::new(InMemoryCaseStore::default()),
                Arc::new(InMemoryStateStore::default()),
            )
        }
    } else {
        info!("Using in-memory stores (default)");
        (
            Arc::new(InMemoryCaseStore::default()),
            Arc::new(InMemoryStateStore::default()),
        )
    };

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry);

    let event_bus = Arc::new(EventBus::default());

    // ── Observability setup ───────────────────────────────────────────────────
    // Always attach an InMemoryObserver for post-run reporting.
    // Optionally also write to a JSONL file for RLHF dataset building.
    let memory_obs = Arc::new(InMemoryObserver::new());
    let jsonl_path = args.windows(2)
        .find(|w| w[0] == "--jsonl")
        .map(|w| w[1].clone());

    let observer: Arc<dyn workflow_engine::Observer> = if let Some(path) = &jsonl_path {
        let jsonl_obs = JsonlObserver::new(path).await?;
        info!("Writing traces to JSONL: {}", path);
        Arc::new(CompositeObserver::new(vec![
            memory_obs.clone(),
            Arc::new(jsonl_obs),
        ]))
    } else {
        memory_obs.clone()
    };

    // Wire up graceful shutdown — Ctrl+C or SIGTERM stops the tick loop after
    // the current tick finishes (in-flight workflows are not abandoned).
    let signal = shutdown_signal();

    if use_parallel {
        // ── Parallel demo: three cases running concurrently ───────────────────
        info!("Running parallel demo (3 approval workflows concurrently)");

        let cases: Vec<Case> = ["parallel_001", "parallel_002", "parallel_003"]
            .iter()
            .map(|key| {
                let mut case = Case::new((*key).into(), "parallel_session".into(), "approval".into());
                case.resource_data = Some(serde_json::json!({
                    "document_id": key,
                    "submitter": "carol",
                    "title": format!("Proposal {}", key)
                }));
                case
            })
            .collect();

        let mut env = SchedulerEnvironment::new("parallel_session", cases)
            .with_execution_mode(ExecutionMode::Parallel);
        let mut scheduler = SchedulerV2::new();
        scheduler.set_observer(observer.clone());

        // Tick 1: all three submit simultaneously
        let result = scheduler
            .tick(
                &mut env,
                &registry,
                Arc::clone(&case_store),
                Arc::clone(&state_store),
                Some(event_bus.as_ref()),
                Some(&signal),
            )
            .await?;
        info!("Parallel tick 1 result: {:?}", result);

        // Inject decisions for all three
        tokio::time::sleep(Duration::from_millis(100)).await;
        for key in &["parallel_001", "parallel_002", "parallel_003"] {
            event_bus.publish(
                key,
                serde_json::json!({"decision": "approved", "reviewer": "dave"}),
            );
        }

        // Tick 2: all three receive decisions in parallel
        let result = scheduler
            .tick(
                &mut env,
                &registry,
                Arc::clone(&case_store),
                Arc::clone(&state_store),
                Some(event_bus.as_ref()),
                Some(&signal),
            )
            .await?;
        info!("Parallel tick 2 result: {:?}", result);

        for key in &["parallel_001", "parallel_002", "parallel_003"] {
            let case = &env.current_case_dict[*key];
            info!(
                "  {}: state={:?}, type={:?}",
                key, case.execution_state, case.finished_type
            );
        }
    } else {
        // ── Sequential demo (default) ─────────────────────────────────────────
        info!("Starting approval workflow demo (Ctrl+C for graceful shutdown)");

        let case_key = "approval_demo_001";
        let mut case = Case::new(case_key.into(), "demo_session".into(), "approval".into());
        case.resource_data = Some(serde_json::json!({
            "document_id": "doc_999",
            "submitter": "carol",
            "title": "Infrastructure Budget 2025"
        }));

        let mut env = SchedulerEnvironment::new("demo_session", vec![case]);
        let mut scheduler = SchedulerV2::new();
        scheduler.set_observer(observer.clone());

        // Tick 1: Submit the document (workflow transitions to WaitReview)
        let result = scheduler
            .tick(
                &mut env,
                &registry,
                Arc::clone(&case_store),
                Arc::clone(&state_store),
                Some(event_bus.as_ref()),
                Some(&signal),
            )
            .await?;

        match result {
            TickResult::ShutdownRequested => {
                info!("Shutdown requested before tick 1, exiting gracefully");
                return Ok(());
            }
            r => info!("After tick 1: {:?}, case state={:?}", r, env.current_case_dict[case_key].execution_state),
        }

        // Simulate a reviewer approving after a short delay
        tokio::time::sleep(Duration::from_millis(100)).await;
        info!("Reviewer approves the document");
        event_bus.publish(
            case_key,
            serde_json::json!({"decision": "approved", "reviewer": "dave"}),
        );

        // Tick 2: Decision arrives — workflow finishes
        let result = scheduler
            .tick(
                &mut env,
                &registry,
                Arc::clone(&case_store),
                Arc::clone(&state_store),
                Some(event_bus.as_ref()),
                Some(&signal),
            )
            .await?;

        match result {
            TickResult::ShutdownRequested => {
                info!("Shutdown requested before tick 2, exiting gracefully");
                return Ok(());
            }
            r => info!("After tick 2: {:?}", r),
        }

        let final_case = &env.current_case_dict[case_key];
        info!(
            "Workflow complete: state={:?}, type={:?}, desc={:?}",
            final_case.execution_state,
            final_case.finished_type,
            final_case.finished_description
        );
    }

    // ── Observability report ──────────────────────────────────────────────────
    // Print a summary of what was collected by the InMemoryObserver.
    let step_records = memory_obs.step_records();
    let workflow_records = memory_obs.workflow_records();
    let llm_records = memory_obs.llm_records();

    info!("--- Observability Summary ---");
    info!("  Steps recorded:     {}", step_records.len());
    info!("  Workflows finished: {}", workflow_records.len());
    info!("  LLM calls recorded: {}", llm_records.len());

    // Show each step with cached/fresh distinction
    for sr in &step_records {
        info!(
            "  step '{}': cached={}, duration={}ms, error={:?}",
            sr.step_name, sr.cached, sr.duration_ms, sr.error
        );
    }

    // Extract RLHF dataset entries (only fresh, successful steps)
    let step_entries = step_dataset(&step_records);
    let wf_entries = workflow_dataset(&workflow_records);
    let llm_entries = llm_dataset(&llm_records);

    info!(
        "  RLHF-ready step entries (non-cached): {}",
        step_entries.len()
    );
    info!("  RLHF-ready workflow entries: {}", wf_entries.len());
    info!("  RLHF-ready LLM call entries: {}", llm_entries.len());

    if let Some(path) = &jsonl_path {
        info!("Full traces written to: {}", path);
    }

    Ok(())
}
