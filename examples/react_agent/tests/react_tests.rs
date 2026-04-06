//! Integration tests for the ReAct agent workflow.
//!
//! Uses a `MockLlmProvider` — no real LLM calls, no database, no network.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use workflow_engine::{
    case::{Case, ExecutionState},
    engine::{SchedulerEnvironment, SchedulerV2, TickResult},
    llm::{LlmProvider, LlmRequest, LlmResponse, LlmUsage, ToolCall},
    store::{InMemoryCaseStore, InMemoryStateStore},
    WorkflowRegistry,
};

use react_agent::register_workflows;

// ── MockLlmProvider ───────────────────────────────────────────────────────────

/// Returns canned responses in sequence.  Panics when exhausted.
struct MockLlmProvider {
    responses: std::sync::Mutex<VecDeque<LlmResponse>>,
    /// Capture all requests so tests can inspect what the LLM received.
    requests: std::sync::Mutex<Vec<LlmRequest>>,
}

impl MockLlmProvider {
    fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: std::sync::Mutex::new(responses.into()),
            requests: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn captured_requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(&self, req: LlmRequest) -> anyhow::Result<LlmResponse> {
        self.requests.lock().unwrap().push(req);
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

fn tool_use_response(tool_name: &str, call_id: &str) -> LlmResponse {
    LlmResponse {
        content: String::new(),
        tool_calls: Some(vec![ToolCall {
            id: call_id.to_string(),
            name: tool_name.to_string(),
            input: serde_json::json!({"path": "/tmp"}),
        }]),
        usage: LlmUsage {
            prompt_tokens: 10,
            completion_tokens: 10,
        },
        finish_reason: "tool_use".to_string(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_stores() -> (Arc<InMemoryCaseStore>, Arc<InMemoryStateStore>) {
    (
        Arc::new(InMemoryCaseStore::default()),
        Arc::new(InMemoryStateStore::default()),
    )
}

fn make_case(key: &str, task: &str) -> Case {
    let mut case = Case::new(key.into(), "test_session".into(), "react_agent".into());
    case.resource_data = Some(serde_json::json!({ "task": task }));
    case
}

/// Tick once, returning the TickResult.
async fn tick_once(
    scheduler: &mut SchedulerV2,
    env: &mut SchedulerEnvironment,
    registry: &WorkflowRegistry,
    cs: Arc<InMemoryCaseStore>,
    ss: Arc<InMemoryStateStore>,
) -> TickResult {
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
        .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: MockLlm returns a direct answer (finish_reason "stop", no tool calls).
/// Workflow should finish in one tick with finished_type = "completed".
#[tokio::test]
async fn workflow_completes_without_tools() {
    let llm = MockLlmProvider::new(vec![stop_response("The answer is 42.")]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm.clone();

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into(), 10);

    let case = make_case("react_no_tools", "What is the answer to life?");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_no_tools", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(&mut sched, &mut env, &registry, cs, ss).await;

    let final_case = &env.current_case_dict["react_no_tools"];
    assert_eq!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should be Finished after a direct answer"
    );
    assert_eq!(
        final_case.finished_type.as_deref(),
        Some("completed"),
        "finished_type should be 'completed'"
    );
    assert_eq!(
        final_case.finished_description.as_deref(),
        Some("The answer is 42."),
        "finished_description should be the LLM answer"
    );
}

/// Test 2: MockLlm returns tool_use on iteration 0, then a final answer on
/// iteration 1.  Workflow should tick twice (Continue then Finished).
#[tokio::test]
async fn workflow_uses_tools_and_continues() {
    // Iteration 0: tool call to "list_directory"
    // Iteration 1: final answer
    let llm = MockLlmProvider::new(vec![
        tool_use_response("list_directory", "call_1"),
        stop_response("Found 3 files in /tmp."),
    ]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm.clone();

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into(), 10);

    let case = make_case("react_with_tools", "List files in /tmp");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_with_tools", vec![case]);
    let mut sched = SchedulerV2::new();

    // Tick 1: should return Continue (tool_use handled, more work to do).
    let tick1 = tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;
    assert!(
        matches!(tick1, TickResult::Executed),
        "First tick should be Executed (workflow ran one iteration)"
    );

    // After tick 1 the case must still be Running (not finished yet).
    let case_after_tick1 = &env.current_case_dict["react_with_tools"];
    assert_eq!(
        case_after_tick1.execution_state,
        ExecutionState::Running,
        "Case should still be Running after tick 1"
    );

    // Tick 2: LLM produces final answer → Finished.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    let final_case = &env.current_case_dict["react_with_tools"];
    assert_eq!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Case should be Finished after tick 2"
    );
    assert_eq!(
        final_case.finished_type.as_deref(),
        Some("completed"),
        "finished_type should be 'completed'"
    );
    assert_eq!(
        final_case.finished_description.as_deref(),
        Some("Found 3 files in /tmp."),
    );
}

/// Test 3: MockLlm always returns tool_use.  Workflow should finish with
/// finished_type = "max_iterations_reached" after max_iterations ticks.
#[tokio::test]
async fn workflow_respects_max_iterations() {
    let max = 3usize;

    // Provide enough tool_use responses to cover all iterations.
    // Each iteration the LLM is called once (tool_use), so we need `max` responses.
    let responses: Vec<LlmResponse> = (0..max)
        .map(|i| tool_use_response("list_directory", &format!("call_{i}")))
        .collect();

    let llm = MockLlmProvider::new(responses);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm.clone();

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into(), max);

    let case = make_case("react_max_iter", "Keep calling tools forever");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_max_iter", vec![case]);
    let mut sched = SchedulerV2::new();

    // Run ticks until the workflow finishes.
    for _ in 0..=max + 1 {
        let result = tick_once(
            &mut sched,
            &mut env,
            &registry,
            Arc::clone(&cs),
            Arc::clone(&ss),
        )
        .await;

        if matches!(result, TickResult::Idle) {
            break;
        }

        if env.current_case_dict["react_max_iter"].execution_state == ExecutionState::Finished {
            break;
        }
    }

    let final_case = &env.current_case_dict["react_max_iter"];
    assert_eq!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should be Finished after max_iterations"
    );
    assert_eq!(
        final_case.finished_type.as_deref(),
        Some("max_iterations_reached"),
        "finished_type should be 'max_iterations_reached'"
    );
}

/// Test 4: Tool results from tick N are included in the LLM prompt in tick N+1.
///
/// In tick 1 the LLM calls a tool.  In tick 2 the LLM should receive the
/// original user message + the assistant tool-call message + the tool-result
/// message (3 messages total).
#[tokio::test]
async fn message_history_is_preserved_across_ticks() {
    let llm = MockLlmProvider::new(vec![
        tool_use_response("list_directory", "call_hist"),
        stop_response("History check passed."),
    ]);

    let llm_ref = Arc::clone(&llm);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm.clone();

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into(), 10);

    let case = make_case("react_history", "Check message history");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_history", vec![case]);
    let mut sched = SchedulerV2::new();

    // Tick 1.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Tick 2.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Inspect what the LLM received in its second call.
    let requests = llm_ref.captured_requests();
    assert_eq!(
        requests.len(),
        2,
        "LLM should have been called exactly twice"
    );

    let second_request = &requests[1];
    // Messages in tick 2 should be:
    //   [0] user: original task
    //   [1] assistant: tool call (list_directory)
    //   [2] user: tool results
    assert_eq!(
        second_request.messages.len(),
        3,
        "Second LLM request should carry 3 messages (user + assistant-tool-call + tool-results), \
         got {}",
        second_request.messages.len()
    );

    assert_eq!(second_request.messages[0].role, "user");
    assert_eq!(second_request.messages[1].role, "assistant");
    assert!(
        second_request.messages[1].tool_calls.is_some(),
        "Second message should be an assistant message with tool calls"
    );
    assert_eq!(second_request.messages[2].role, "user");
    assert!(
        second_request.messages[2].tool_results.is_some(),
        "Third message should carry tool results"
    );
}
