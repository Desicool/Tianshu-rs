/// Tests for the approval workflow — pure in-memory, no database, no LLM calls.
use std::sync::Arc;
use workflow_engine::{
    case::{Case, ExecutionState},
    engine::{SchedulerEnvironment, SchedulerV2},
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
    WorkflowRegistry,
};

use approval_workflow::register_workflows;

fn make_stores() -> (Arc<InMemoryCaseStore>, Arc<InMemoryStateStore>) {
    (
        Arc::new(InMemoryCaseStore::default()),
        Arc::new(InMemoryStateStore::default()),
    )
}

fn make_approval_case(key: &str) -> Case {
    let mut case = Case::new(key.into(), "approval_session".into(), "approval".into());
    case.resource_data = Some(serde_json::json!({
        "document_id": "doc_001",
        "submitter": "bob",
        "title": "Q4 Budget Proposal"
    }));
    case
}

async fn tick(
    scheduler: &mut SchedulerV2,
    env: &mut SchedulerEnvironment,
    registry: &WorkflowRegistry,
    cs: Arc<InMemoryCaseStore>,
    ss: Arc<InMemoryStateStore>,
) {
    scheduler
        .tick(
            env,
            registry,
            cs as Arc<dyn workflow_engine::store::CaseStore>,
            ss as Arc<dyn workflow_engine::store::StateStore>,
            None,
            None,
        )
        .await
        .unwrap();
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn approval_workflow_registered() {
    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry);
    assert!(registry.registered_codes().contains(&"approval".to_string()));
}

#[tokio::test]
async fn workflow_starts_running_then_waits_for_review() {
    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry);

    let case = make_approval_case("case_run_wait");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::new("s1", vec![case]);
    let mut sched = SchedulerV2::new();

    // First tick: submits the document, then enters Waiting for review
    tick(&mut sched, &mut env, &registry, cs, ss).await;

    let stored = env.current_case_dict.get("case_run_wait").unwrap();
    assert_eq!(stored.execution_state, ExecutionState::Waiting);
}

#[tokio::test]
async fn workflow_completes_approved() {
    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry);

    let case = make_approval_case("case_approve");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::new("s2", vec![case]);
    let mut sched = SchedulerV2::new();

    // Tick 1: submit → waiting
    tick(&mut sched, &mut env, &registry, Arc::clone(&cs), Arc::clone(&ss)).await;
    assert_eq!(
        env.current_case_dict["case_approve"].execution_state,
        ExecutionState::Waiting
    );

    // Simulate orchestrator injecting the decision into resource_data
    if let Some(c) = env.current_case_dict.get_mut("case_approve") {
        c.resource_data = Some(serde_json::json!({
            "document_id": "doc_001",
            "decision": "approved",
            "reviewer": "alice"
        }));
        c.mc_run(); // wake up the case so the engine executes it
    }

    // Tick 2: decision found → approved → finished
    tick(&mut sched, &mut env, &registry, Arc::clone(&cs), Arc::clone(&ss)).await;

    let final_case = &env.current_case_dict["case_approve"];
    assert_eq!(final_case.execution_state, ExecutionState::Finished);
    assert_eq!(final_case.finished_type.as_deref(), Some("approved"));
}

#[tokio::test]
async fn workflow_completes_rejected() {
    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry);

    let case = make_approval_case("case_reject");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::new("s3", vec![case]);
    let mut sched = SchedulerV2::new();

    // Tick 1: submit → waiting
    tick(&mut sched, &mut env, &registry, Arc::clone(&cs), Arc::clone(&ss)).await;

    // Inject rejection
    if let Some(c) = env.current_case_dict.get_mut("case_reject") {
        c.resource_data = Some(serde_json::json!({
            "decision": "rejected",
            "reason": "Budget too high"
        }));
        c.mc_run();
    }

    // Tick 2: rejection processed
    tick(&mut sched, &mut env, &registry, Arc::clone(&cs), Arc::clone(&ss)).await;

    let final_case = &env.current_case_dict["case_reject"];
    assert_eq!(final_case.execution_state, ExecutionState::Finished);
    assert_eq!(final_case.finished_type.as_deref(), Some("rejected"));
    assert_eq!(
        final_case.finished_description.as_deref(),
        Some("Budget too high")
    );
}

#[tokio::test]
async fn workflow_stays_waiting_without_decision() {
    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry);

    let case = make_approval_case("case_no_decision");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::new("s4", vec![case]);
    let mut sched = SchedulerV2::new();

    tick(&mut sched, &mut env, &registry, Arc::clone(&cs), Arc::clone(&ss)).await;
    // Still waiting — no decision injected
    assert_eq!(
        env.current_case_dict["case_no_decision"].execution_state,
        ExecutionState::Waiting
    );

    // Second tick without injecting decision — still waiting
    tick(&mut sched, &mut env, &registry, Arc::clone(&cs), Arc::clone(&ss)).await;
    assert_eq!(
        env.current_case_dict["case_no_decision"].execution_state,
        ExecutionState::Waiting
    );
}
