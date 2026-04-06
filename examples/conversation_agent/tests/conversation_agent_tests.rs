//! Integration tests for the ConversationWorkflow.
//!
//! Uses a `MockLlmProvider` — no real LLM calls, no database, no network.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use workflow_engine::{
    case::{Case, ExecutionState},
    engine::{SchedulerEnvironment, SchedulerV2, TickResult},
    llm::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmUsage, ToolCall},
    store::{InMemoryCaseStore, InMemoryStateStore},
    WorkflowRegistry,
};

use conversation_agent::register_workflows;

// ── MockLlmProvider ───────────────────────────────────────────────────────────

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

fn tool_use_response(tool_name: &str, call_id: &str, input: serde_json::Value) -> LlmResponse {
    LlmResponse {
        content: String::new(),
        tool_calls: Some(vec![ToolCall {
            id: call_id.to_string(),
            name: tool_name.to_string(),
            input,
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

fn make_case(key: &str) -> Case {
    Case::new(key.into(), "test_session".into(), "conversation".into())
}

fn make_case_with_message(key: &str, message: &str) -> Case {
    let mut case = Case::new(key.into(), "test_session".into(), "conversation".into());
    case.resource_data = Some(json!({ "user_message": message }));
    case
}

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

/// Read conv_messages from state store for a given case key.
async fn read_messages(ss: &InMemoryStateStore, case_key: &str) -> Vec<LlmMessage> {
    use workflow_engine::store::StateStore;
    let entry = ss.get(case_key, "wf_state_conv_messages").await.unwrap();
    match entry {
        Some(e) => serde_json::from_str(&e.data).unwrap_or_default(),
        None => vec![],
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: No user_message → workflow waits, no messages recorded.
#[tokio::test]
async fn workflow_waits_when_no_user_message() {
    let llm = MockLlmProvider::new(vec![]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    // Case with no resource_data.
    let case = make_case("conv_no_input");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_no_input", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    let final_case = &env.current_case_dict["conv_no_input"];
    // Should NOT be finished — workflow is waiting.
    assert_ne!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should not finish when there is no user message"
    );

    // No messages should be recorded.
    let messages = read_messages(&ss, "conv_no_input").await;
    assert!(
        messages.is_empty(),
        "No messages should be recorded when no user input"
    );
}

/// Test 2: Single user message → LLM replies → state has user + assistant messages.
#[tokio::test]
async fn workflow_processes_user_message() {
    let llm = MockLlmProvider::new(vec![stop_response("Hi there!")]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    let case = make_case_with_message("conv_first_msg", "Hello!");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_first_msg", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    // Workflow should be Waiting (not Finished) — awaiting next turn.
    let final_case = &env.current_case_dict["conv_first_msg"];
    assert_ne!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should be Waiting after first message, not Finished"
    );

    // State should contain user message + assistant reply.
    let messages = read_messages(&ss, "conv_first_msg").await;
    assert!(
        messages.len() >= 2,
        "Should have at least user + assistant messages, got {}",
        messages.len()
    );

    let user_msg = messages.iter().find(|m| m.role == "user");
    assert!(user_msg.is_some(), "Should have a user message");
    assert_eq!(user_msg.unwrap().content, "Hello!");

    let asst_msg = messages.iter().rev().find(|m| m.role == "assistant");
    assert!(asst_msg.is_some(), "Should have an assistant message");
    assert_eq!(asst_msg.unwrap().content, "Hi there!");
}

/// Test 3: Multi-turn conversation — history accumulates correctly.
#[tokio::test]
async fn workflow_handles_multi_turn() {
    let llm = MockLlmProvider::new(vec![
        stop_response("The answer is 4."),
        stop_response("You're welcome!"),
    ]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    // Turn 1.
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id(
        "s_multi",
        vec![make_case_with_message("conv_multi", "What is 2+2?")],
    );
    let mut sched = SchedulerV2::new();

    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    let messages_after_turn1 = read_messages(&ss, "conv_multi").await;
    let asst1 = messages_after_turn1
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .expect("Should have assistant message after turn 1");
    assert_eq!(asst1.content, "The answer is 4.");

    // Turn 2: update resource_data on the env case.
    let conv_case = env.current_case_dict.get_mut("conv_multi").unwrap();
    conv_case.resource_data = Some(json!({ "user_message": "Thanks!" }));

    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    let messages_after_turn2 = read_messages(&ss, "conv_multi").await;
    // Should have: user(Q1), assistant(A1), user(Thanks!), assistant(Welcome)
    assert!(
        messages_after_turn2.len() >= 4,
        "Should have at least 4 messages after 2 turns, got {}",
        messages_after_turn2.len()
    );

    let asst2 = messages_after_turn2
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .expect("Should have assistant message after turn 2");
    assert_eq!(asst2.content, "You're welcome!");

    // Verify turn 1 messages still present.
    let user_msgs: Vec<_> = messages_after_turn2
        .iter()
        .filter(|m| m.role == "user")
        .collect();
    assert!(user_msgs.len() >= 2, "Should have at least 2 user messages");
}

/// Test 4: Submitting the same message twice — the second tick should be ignored.
/// The conv_messages history should not grow after the duplicate.
#[tokio::test]
async fn workflow_ignores_duplicate_message() {
    // Only one LLM call should happen — the duplicate must not trigger a second.
    let llm = MockLlmProvider::new(vec![stop_response("First reply.")]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    let case = make_case_with_message("conv_dup", "Hello!");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_dup", vec![case]);
    let mut sched = SchedulerV2::new();

    // First tick — processes the message.
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    let messages_after_first = read_messages(&ss, "conv_dup").await;
    assert_eq!(
        messages_after_first.len(),
        2,
        "Should have user + assistant after first tick"
    );

    // Second tick with the SAME user_message — must be a no-op.
    // (resource_data was not changed; same "Hello!" is still set)
    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    let messages_after_second = read_messages(&ss, "conv_dup").await;
    assert_eq!(
        messages_after_second.len(),
        2,
        "History should not grow after a duplicate message (got {})",
        messages_after_second.len()
    );
    // If MockLlmProvider panics here it means a second LLM call was made —
    // that would also prove the guard failed.
}

/// Test 5: Workflow invokes a tool when requested.
#[tokio::test]
async fn workflow_uses_tools_when_needed() {
    use std::io::Write as IoWrite;

    // Write a temp file the read_file tool can read.
    let tmp_path = "/tmp/conv_agent_test.txt";
    {
        let mut f = std::fs::File::create(tmp_path).unwrap();
        f.write_all(b"hello from test file").unwrap();
    }

    let llm = MockLlmProvider::new(vec![
        // First: LLM requests a tool call.
        tool_use_response("read_file", "call_rf_1", json!({ "path": tmp_path })),
        // Second: LLM receives tool result and gives final answer.
        stop_response("The file contains: hello from test file"),
    ]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    let case = make_case_with_message("conv_tools", &format!("Read the file {}", tmp_path));
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_tools", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(
        &mut sched,
        &mut env,
        &registry,
        Arc::clone(&cs),
        Arc::clone(&ss),
    )
    .await;

    let messages = read_messages(&ss, "conv_tools").await;

    // Should have: user + assistant-with-tool-call + tool-results + final-assistant.
    assert!(
        messages.len() >= 3,
        "Should have at least 3 messages after tool use, got {}",
        messages.len()
    );

    let last_asst = messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && !m.content.is_empty())
        .expect("Should have a final assistant text response");
    assert!(
        last_asst.content.contains("hello from test file"),
        "Final response should reference file content, got: {}",
        last_asst.content
    );
}

/// Test 5: register_workflows registers the "conversation" workflow.
#[tokio::test]
async fn workflow_registers_correctly() {
    let llm = MockLlmProvider::new(vec![]);
    let llm_dyn: Arc<dyn workflow_engine::llm::LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock-model".into());

    assert!(
        registry
            .registered_codes()
            .contains(&"conversation".to_string()),
        "Registry should contain 'conversation' workflow"
    );
}
