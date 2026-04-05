//! Approval workflow orchestrator.
//!
//! Demonstrates running the workflow engine with in-memory stores.
//! Pass `--postgres <DATABASE_URL>` to use PostgreSQL instead.
//! Pass `--parallel` to run multiple workflows in parallel mode.
//!
//! Usage:
//!   cargo run -p approval_workflow
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

    Ok(())
}
