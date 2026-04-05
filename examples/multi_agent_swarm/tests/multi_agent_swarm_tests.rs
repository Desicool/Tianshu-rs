//! Integration tests for the multi-agent swarm workflow.
//!
//! Uses a `MockLlmProvider` — no real LLM calls, no database, no network.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use workflow_engine::{
    case::{Case, ExecutionState},
    engine::{ExecutionMode, SchedulerEnvironment, SchedulerV2},
    llm::{LlmProvider, LlmRequest, LlmResponse, LlmUsage},
    store::{InMemoryCaseStore, InMemoryStateStore, StateStore},
    WorkflowRegistry,
};

use multi_agent_swarm::register_workflows;

// ── MockLlmProvider ───────────────────────────────────────────────────────────

/// Returns canned responses in sequence. Panics when exhausted.
struct MockLlmProvider {
    responses: std::sync::Mutex<VecDeque<LlmResponse>>,
}

impl MockLlmProvider {
    fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: std::sync::Mutex::new(responses.into()),
        })
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(&self, _req: LlmRequest) -> anyhow::Result<LlmResponse> {
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("MockLlmProvider: no more canned responses"))
    }
}

// ── Response helpers ──────────────────────────────────────────────────────────

fn stop_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: content.to_string(),
        tool_calls: None,
        usage: LlmUsage {
            prompt_tokens: 10,
            completion_tokens: 10,
        },
        finish_reason: "stop".to_string(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_stores() -> (Arc<InMemoryCaseStore>, Arc<InMemoryStateStore>) {
    (
        Arc::new(InMemoryCaseStore::default()),
        Arc::new(InMemoryStateStore::default()),
    )
}

async fn tick_once(
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

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: Leader plans and assigns tasks.
/// Mock LLM returns a planning response with 2 subtasks as JSON.
/// After 2 ticks, session state contains worker_manifest and worker_task_{id}.
#[tokio::test]
async fn leader_plans_and_assigns_tasks() {
    // The leader LLM call returns a JSON array of 2 subtasks.
    let planning_json = r#"[{"id":"w1","description":"Task one: do something"},{"id":"w2","description":"Task two: do something else"}]"#;
    let llm = MockLlmProvider::new(vec![stop_response(planning_json)]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    let session_id = "test_session_leader";
    let mut leader_case = Case::new("leader".into(), session_id.into(), "swarm_leader".into());
    leader_case.resource_data = Some(json!({"task": "Break this task into 2 subtasks"}));

    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id(session_id, vec![leader_case]);
    let mut sched = SchedulerV2::new();

    // Tick 1: Leader calls LLM to plan tasks, assigns them to session state.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Tick 2: Leader signals workers_spawned.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Verify session state contains worker_manifest.
    let manifest_entry = ss
        .get_session(session_id, "wf_sess_state_worker_manifest")
        .await
        .unwrap();
    assert!(
        manifest_entry.is_some(),
        "worker_manifest should be set in session state"
    );

    let manifest: Vec<String> = serde_json::from_str(&manifest_entry.unwrap().data).unwrap();
    assert_eq!(manifest.len(), 2, "manifest should have 2 worker IDs");

    // Verify each worker_task is in session state.
    for id in &manifest {
        let task_entry = ss
            .get_session(session_id, &format!("wf_sess_state_worker_task_{}", id))
            .await
            .unwrap();
        assert!(
            task_entry.is_some(),
            "worker_task_{} should be set in session state",
            id
        );
        let task_str: String = serde_json::from_str(&task_entry.unwrap().data).unwrap();
        assert!(
            !task_str.is_empty(),
            "task description for {} should not be empty",
            id
        );
    }
}

/// Test 2: Worker reads and executes task.
/// Pre-populate session state with worker_task_w1.
/// After worker finishes, verify worker_result_w1 is set.
#[tokio::test]
async fn worker_reads_and_executes_task() {
    let session_id = "test_session_worker";

    // Worker LLM returns a direct answer (no tool use).
    let llm = MockLlmProvider::new(vec![stop_response("The answer to task w1 is: completed!")]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    let (cs, ss) = make_stores();

    // Pre-populate session state with the worker task.
    ss.save_session(
        session_id,
        "wf_sess_state_worker_task_w1",
        r#""Count to 3""#,
    )
    .await
    .unwrap();

    let mut worker_case = Case::new("worker_w1".into(), session_id.into(), "swarm_worker".into());
    worker_case.resource_data = Some(json!({"worker_id": "w1"}));

    let mut env = SchedulerEnvironment::from_session_id(session_id, vec![worker_case]);
    let mut sched = SchedulerV2::new();

    // Tick until the worker finishes.
    for _ in 0..5 {
        tick_once(
            &mut sched,
            &mut env,
            &registry,
            Arc::clone(&cs),
            Arc::clone(&ss),
        )
        .await;

        if env.current_case_dict["worker_w1"].execution_state == ExecutionState::Finished {
            break;
        }
    }

    let worker_case = &env.current_case_dict["worker_w1"];
    assert_eq!(
        worker_case.execution_state,
        ExecutionState::Finished,
        "Worker case should be Finished"
    );

    // Worker finish clears case state but session state (worker_result_w1) should persist.
    let result_entry = ss
        .get_session(session_id, "wf_sess_state_worker_result_w1")
        .await
        .unwrap();
    assert!(
        result_entry.is_some(),
        "worker_result_w1 should be set in session state"
    );
    let result: String = serde_json::from_str(&result_entry.unwrap().data).unwrap();
    assert!(!result.is_empty(), "worker result should not be empty");
}

/// Test 3: Full swarm runs end-to-end.
/// Leader plans, workers execute, leader synthesizes.
#[tokio::test]
async fn full_swarm_runs_end_to_end() {
    let session_id = "test_session_full_swarm";

    let planning_json =
        r#"[{"id":"w1","description":"Subtask one"},{"id":"w2","description":"Subtask two"}]"#;

    // Responses in order:
    // 1. Leader: plan_tasks LLM call -> planning_json
    // 2. Worker w1: execute_task LLM call -> direct answer
    // 3. Worker w2: execute_task LLM call -> direct answer
    // 4. Leader: synthesize LLM call -> final answer
    let llm = MockLlmProvider::new(vec![
        stop_response(planning_json),
        stop_response("Worker w1 result: done subtask one"),
        stop_response("Worker w2 result: done subtask two"),
        stop_response("Final synthesized answer from all workers"),
    ]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    let (cs, ss) = make_stores();

    let mut leader_case = Case::new("leader".into(), session_id.into(), "swarm_leader".into());
    leader_case.resource_data = Some(json!({"task": "Do a multi-part task"}));

    let mut env = SchedulerEnvironment::from_session_id(session_id, vec![leader_case]);
    env.execution_mode = ExecutionMode::Parallel;
    let mut sched = SchedulerV2::new();

    // Tick 1: Leader plans and assigns tasks (LLM call for plan_tasks).
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Tick 2: Leader sets workers_spawned = true, returns Continue.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Between ticks: check if workers need spawning.
    let workers_spawned_entry = ss
        .get_session(session_id, "wf_sess_state_workers_spawned")
        .await
        .unwrap();
    let workers_spawned: bool = workers_spawned_entry
        .map(|e| serde_json::from_str(&e.data).unwrap_or(false))
        .unwrap_or(false);

    assert!(
        workers_spawned,
        "workers_spawned should be true after tick 2"
    );

    // Spawn worker cases by reading the manifest.
    let manifest_entry = ss
        .get_session(session_id, "wf_sess_state_worker_manifest")
        .await
        .unwrap()
        .expect("worker_manifest should be set");
    let manifest: Vec<String> = serde_json::from_str(&manifest_entry.data).unwrap();
    assert_eq!(manifest.len(), 2);

    for id in &manifest {
        let mut worker = Case::new(
            format!("worker_{}", id),
            session_id.into(),
            "swarm_worker".into(),
        );
        worker.resource_data = Some(json!({"worker_id": id}));
        env.current_case_dict
            .insert(worker.case_key.clone(), worker);
    }

    // Tick 3: Workers execute + report results; Leader polls (Continue waiting).
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Tick 4: Leader detects all results done, synthesizes, finishes.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Allow extra ticks in case synthesis needs another round.
    for _ in 0..3 {
        if env.current_case_dict["leader"].execution_state == ExecutionState::Finished {
            break;
        }
        tick_once(
            &mut sched,
            &mut env,
            &registry,
            Arc::clone(&cs),
            Arc::clone(&ss),
        )
        .await;
    }

    let final_leader = &env.current_case_dict["leader"];
    assert_eq!(
        final_leader.execution_state,
        ExecutionState::Finished,
        "Leader case should be Finished after full swarm completes"
    );
    assert_eq!(
        final_leader.finished_type.as_deref(),
        Some("completed"),
        "finished_type should be 'completed'"
    );
    assert!(
        final_leader.finished_description.is_some(),
        "finished_description should be set"
    );
}
