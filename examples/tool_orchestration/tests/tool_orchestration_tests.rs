//! Integration tests for the tool orchestration workflow.
//!
//! Uses `MockLlmProvider` and `MockTool` — no real LLM calls, no database, no
//! network.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;

use workflow_engine::{
    case::{Case, ExecutionState},
    engine::{SchedulerEnvironment, SchedulerV2, TickResult},
    llm::{LlmProvider, LlmRequest, LlmResponse, LlmUsage, ToolCall},
    store::{InMemoryCaseStore, InMemoryStateStore},
    WorkflowRegistry,
};

use tool_orchestration::register_workflows;

// ── Mock LLM ─────────────────────────────────────────────────────────────────

/// Returns canned responses in sequence.  Panics when exhausted.
struct MockLlmProvider {
    responses: Mutex<VecDeque<LlmResponse>>,
}

impl MockLlmProvider {
    fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
        })
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse> {
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("MockLlmProvider: no more canned responses"))
    }
}

// ── Mock Tool ─────────────────────────────────────────────────────────────────

use example_tools::{Tool, ToolSafety};

struct MockTool {
    tool_name: String,
    tool_safety: ToolSafety,
    tool_result: String,
    call_count: Arc<AtomicUsize>,
}

impl MockTool {
    fn new(
        name: impl Into<String>,
        safety: ToolSafety,
        result: impl Into<String>,
        counter: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            tool_name: name.into(),
            tool_safety: safety,
            tool_result: result.into(),
            call_count: counter,
        }
    }
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Mock tool for testing"
    }

    fn safety(&self) -> ToolSafety {
        self.tool_safety
    }

    fn input_schema(&self) -> JsonValue {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _input: JsonValue) -> Result<String> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.tool_result.clone())
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

fn tool_use_response(calls: Vec<(&str, &str)>) -> LlmResponse {
    LlmResponse {
        content: String::new(),
        tool_calls: Some(
            calls
                .into_iter()
                .map(|(name, id)| ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    input: serde_json::json!({}),
                })
                .collect(),
        ),
        usage: LlmUsage {
            prompt_tokens: 10,
            completion_tokens: 10,
        },
        finish_reason: "tool_use".to_string(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

use example_tools::ToolRegistry;
use std::collections::HashMap;
use tool_orchestration::{
    stages::{
        ExecuteConcurrentStage, ExecuteExclusiveStage, FeedResultsStage, PlanToolsStage,
        SynthesizeStage,
    },
    workflows::{OrchStage, ToolOrchestrationWorkflow},
};
use workflow_engine::stage::StageBase;

/// Register a test workflow into `registry` using the provided LLM and tools.
///
/// All stage dependencies are `Arc`-wrapped so the `Fn` factory closure can
/// clone them on every call without moving them.
fn register_test_workflow(
    registry: &mut WorkflowRegistry,
    llm: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
) {
    let model = "mock-model".to_string();

    // Wrap shared state so we can clone into the Fn closure.
    let llm = Arc::clone(&llm);
    let tools = Arc::clone(&tools);

    registry.register("tool_orchestration", move |_case| {
        let mut stage_map: HashMap<OrchStage, Box<dyn StageBase<OrchStage>>> = HashMap::new();

        stage_map.insert(
            OrchStage::PlanTools,
            Box::new(PlanToolsStage {
                llm: Arc::clone(&llm),
                model: model.clone(),
                tools: Arc::clone(&tools),
            }),
        );
        stage_map.insert(
            OrchStage::ExecuteConcurrent,
            Box::new(ExecuteConcurrentStage {
                tools: Arc::clone(&tools),
            }),
        );
        stage_map.insert(
            OrchStage::ExecuteExclusive,
            Box::new(ExecuteExclusiveStage {
                tools: Arc::clone(&tools),
            }),
        );
        stage_map.insert(OrchStage::FeedResults, Box::new(FeedResultsStage));
        stage_map.insert(
            OrchStage::Synthesize,
            Box::new(SynthesizeStage {
                llm: Arc::clone(&llm),
                model: model.clone(),
            }),
        );

        Box::new(ToolOrchestrationWorkflow {
            stages: Arc::new(stage_map),
            start_stage: OrchStage::PlanTools,
        })
    });
}

fn make_stores() -> (Arc<InMemoryCaseStore>, Arc<InMemoryStateStore>) {
    (
        Arc::new(InMemoryCaseStore::default()),
        Arc::new(InMemoryStateStore::default()),
    )
}

fn make_case(key: &str, task: &str) -> Case {
    let mut case = Case::new(
        key.into(),
        "test_session".into(),
        "tool_orchestration".into(),
    );
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

/// Test 1: LLM returns "stop" immediately — no tools needed.
/// Workflow should complete in one tick with `finished_type = "completed"`.
#[tokio::test]
async fn workflow_completes_with_no_tools_needed() {
    let llm = MockLlmProvider::new(vec![stop_response("Direct answer: 42")]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let tools = Arc::new(ToolRegistry::new());

    let mut registry = WorkflowRegistry::new();
    register_test_workflow(&mut registry, Arc::clone(&llm_dyn), Arc::clone(&tools));

    let case = make_case("orch_no_tools", "What is 6 x 7?");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_no_tools", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(&mut sched, &mut env, &registry, cs, ss).await;

    let final_case = &env.current_case_dict["orch_no_tools"];
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
        Some("Direct answer: 42"),
        "finished_description should be the LLM answer"
    );
}

/// Test 2: LLM returns tool_use with 2 ConcurrentSafe calls, then "stop".
/// Both tools should be called and their results fed back.
#[tokio::test]
async fn workflow_executes_concurrent_tools() {
    let counter_a = Arc::new(AtomicUsize::new(0));
    let counter_b = Arc::new(AtomicUsize::new(0));

    let mut tools = ToolRegistry::new();
    tools.register(MockTool::new(
        "read_a",
        ToolSafety::ConcurrentSafe,
        "content_a",
        Arc::clone(&counter_a),
    ));
    tools.register(MockTool::new(
        "read_b",
        ToolSafety::ConcurrentSafe,
        "content_b",
        Arc::clone(&counter_b),
    ));
    let tools = Arc::new(tools);

    let llm = MockLlmProvider::new(vec![
        tool_use_response(vec![("read_a", "call_a"), ("read_b", "call_b")]),
        stop_response("Both files read."),
    ]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_test_workflow(&mut registry, Arc::clone(&llm_dyn), Arc::clone(&tools));

    let case = make_case("orch_concurrent", "Read two files concurrently");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_concurrent", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(&mut sched, &mut env, &registry, cs, ss).await;

    let final_case = &env.current_case_dict["orch_concurrent"];
    assert_eq!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should be Finished"
    );
    assert_eq!(counter_a.load(Ordering::SeqCst), 1, "read_a called once");
    assert_eq!(counter_b.load(Ordering::SeqCst), 1, "read_b called once");
}

/// Test 3: LLM returns tool_use with 2 Exclusive calls, then "stop".
/// Both exclusive tools should have been called sequentially.
#[tokio::test]
async fn workflow_executes_exclusive_tools_sequentially() {
    let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    struct OrderedTool {
        name: String,
        order: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Tool for OrderedTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "Ordered exclusive tool"
        }
        fn safety(&self) -> ToolSafety {
            ToolSafety::Exclusive
        }
        fn input_schema(&self) -> JsonValue {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _input: JsonValue) -> Result<String> {
            self.order.lock().unwrap().push(self.name.clone());
            Ok(format!("done:{}", self.name))
        }
    }

    let mut tools = ToolRegistry::new();
    tools.register(OrderedTool {
        name: "cmd_1".to_string(),
        order: Arc::clone(&order),
    });
    tools.register(OrderedTool {
        name: "cmd_2".to_string(),
        order: Arc::clone(&order),
    });
    let tools = Arc::new(tools);

    let llm = MockLlmProvider::new(vec![
        tool_use_response(vec![("cmd_1", "ex_1"), ("cmd_2", "ex_2")]),
        stop_response("Both commands executed."),
    ]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_test_workflow(&mut registry, Arc::clone(&llm_dyn), Arc::clone(&tools));

    let case = make_case("orch_exclusive", "Run two exclusive commands");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_exclusive", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(&mut sched, &mut env, &registry, cs, ss).await;

    let final_case = &env.current_case_dict["orch_exclusive"];
    assert_eq!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should be Finished"
    );

    // Verify both commands were called in order.
    let execution_order = order.lock().unwrap().clone();
    assert_eq!(execution_order, vec!["cmd_1", "cmd_2"]);
}

/// Test 4: LLM returns tool_use → tool_use → stop (multiple tool rounds).
/// Workflow should handle both rounds correctly and finish.
#[tokio::test]
async fn workflow_handles_multiple_tool_rounds() {
    let counter = Arc::new(AtomicUsize::new(0));

    let mut tools = ToolRegistry::new();
    tools.register(MockTool::new(
        "search",
        ToolSafety::ConcurrentSafe,
        "search_result",
        Arc::clone(&counter),
    ));
    let tools = Arc::new(tools);

    // Round 1: tool_use, Round 2: tool_use, Round 3: stop.
    let llm = MockLlmProvider::new(vec![
        tool_use_response(vec![("search", "s_1")]),
        tool_use_response(vec![("search", "s_2")]),
        stop_response("Final synthesized answer after two rounds."),
    ]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_test_workflow(&mut registry, Arc::clone(&llm_dyn), Arc::clone(&tools));

    let case = make_case("orch_multi_round", "Multi-round tool search");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_multi_round", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(&mut sched, &mut env, &registry, cs, ss).await;

    let final_case = &env.current_case_dict["orch_multi_round"];
    assert_eq!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should be Finished after multiple rounds"
    );
    assert_eq!(
        final_case.finished_type.as_deref(),
        Some("completed"),
        "finished_type should be 'completed'"
    );
    // Tool was called twice (once per round).
    assert_eq!(counter.load(Ordering::SeqCst), 2, "search called twice");
}

/// Test 5: Synthesize fallback — when PlanTools saves an assistant message with
/// empty content, SynthesizeStage falls back to an extra LLM synthesis call.
#[tokio::test]
async fn synthesize_falls_back_to_llm_when_no_assistant_message() {
    // PlanTools will return tool_use, then "stop" with empty content.
    // SynthesizeStage should detect the empty assistant message and call the LLM
    // one more time for a synthesis response.
    let counter = Arc::new(AtomicUsize::new(0));

    let mut tools = ToolRegistry::new();
    tools.register(MockTool::new(
        "read_file",
        ToolSafety::ConcurrentSafe,
        "file_content",
        Arc::clone(&counter),
    ));
    let tools = Arc::new(tools);

    // Sequence: tool_use → stop with empty content → synthesis response
    let llm = MockLlmProvider::new(vec![
        tool_use_response(vec![("read_file", "rf_1")]),
        // PlanTools gets "stop" with empty content; appends empty assistant msg
        LlmResponse {
            content: String::new(),
            tool_calls: None,
            usage: LlmUsage { prompt_tokens: 5, completion_tokens: 0 },
            finish_reason: "stop".to_string(),
        },
        // SynthesizeStage fallback call
        stop_response("Synthesized: file_content processed successfully."),
    ]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;

    let mut registry = WorkflowRegistry::new();
    register_test_workflow(&mut registry, Arc::clone(&llm_dyn), Arc::clone(&tools));

    let case = make_case("orch_synth_fallback", "Read a file");
    let (cs, ss) = make_stores();
    let mut env = SchedulerEnvironment::from_session_id("s_synth_fallback", vec![case]);
    let mut sched = SchedulerV2::new();

    tick_once(&mut sched, &mut env, &registry, cs, ss).await;

    let final_case = &env.current_case_dict["orch_synth_fallback"];
    assert_eq!(
        final_case.execution_state,
        ExecutionState::Finished,
        "Workflow should be Finished"
    );
    assert_eq!(
        final_case.finished_description.as_deref(),
        Some("Synthesized: file_content processed successfully."),
        "Should use the fallback synthesis response"
    );
}

/// Test 6: Workflow registers correctly under the default code.
#[tokio::test]
async fn workflow_registers_correctly() {
    let llm = MockLlmProvider::new(vec![]);
    let llm_dyn: Arc<dyn LlmProvider> = llm;
    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, llm_dyn, "mock".into());
    assert!(registry
        .registered_codes()
        .contains(&"tool_orchestration".to_string()));
}

/// Test 7: PlanToolsStage must use ctx.step("plan_tools_step", ...) so that a
/// pre-seeded checkpoint prevents the LLM from being called on replay.
///
/// This test is the RED phase of TDD for the checkpoint fix.  Before the fix,
/// PlanToolsStage calls the LLM unconditionally (no ctx.step), so the mock LLM
/// will be called even though a cached response is already in the state store.
/// After the fix, the LLM call count must be zero.
#[tokio::test]
async fn plan_tools_stage_uses_checkpoint_on_replay() {
    use workflow_engine::{
        context::WorkflowContext,
        llm::{LlmResponse, LlmUsage},
        store::{InMemoryCaseStore, InMemoryStateStore, StateStore},
    };

    // --- Set up stores and pre-seed a "stop" checkpoint so PlanToolsStage
    //     thinks the LLM already ran for this case.
    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());

    let case_key = "ck_checkpoint_test";

    // The checkpoint key used by ctx.step("plan_tools_step_0") on round 0 is
    // "wf_plan_tools_step_0" (WorkflowContext::checkpoint_step_key prefixes with "wf_").
    let cached_response = LlmResponse {
        content: "Cached direct answer".to_string(),
        tool_calls: None,
        usage: LlmUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
        },
        finish_reason: "stop".to_string(),
    };
    let cached_json = serde_json::to_string(&cached_response).unwrap();
    ss.save(case_key, "wf_plan_tools_step_0", &cached_json)
        .await
        .unwrap();

    // --- Build a counting mock LLM.  After the fix, complete() must NOT be called.
    struct CountingLlm {
        count: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl LlmProvider for CountingLlm {
        async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(LlmResponse {
                content: "SHOULD NOT BE CALLED".to_string(),
                tool_calls: None,
                usage: LlmUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                },
                finish_reason: "stop".to_string(),
            })
        }
    }

    let call_count = Arc::new(AtomicUsize::new(0));
    let llm: Arc<dyn LlmProvider> = Arc::new(CountingLlm {
        count: Arc::clone(&call_count),
    });

    // --- Build a WorkflowContext with the pre-seeded state store.
    let case = make_case(case_key, "What is 6x7?");
    let mut ctx = WorkflowContext::new(
        case,
        cs as Arc<dyn workflow_engine::store::CaseStore>,
        ss as Arc<dyn workflow_engine::store::StateStore>,
    );

    // --- Execute PlanToolsStage.
    let tools = Arc::new(ToolRegistry::new());
    let stage = PlanToolsStage {
        llm,
        model: "mock".to_string(),
        tools,
    };
    let _outcome = stage.execute(&mut ctx).await.unwrap();

    // ASSERTION: LLM must not have been called because the checkpoint was hit.
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        0,
        "PlanToolsStage must use ctx.step() so that a cached checkpoint \
         prevents the LLM from being called on replay"
    );
}
