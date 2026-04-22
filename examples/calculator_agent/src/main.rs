//! Calculator Agent — Phase 4 agent-first architecture demo.
//!
//! Demonstrates the full agent lifecycle:
//!   1. `Agent::root()` creates a coordinator agent + backing Case.
//!   2. CoordinatorAgent receives "calculate 3 + 5 * 2" via `resource_data`.
//!   3. On tick 1, coordinator uses `converse()` with full tool access and a
//!      MockLlmProvider, calls `multiply` first, then spawns an ArithmeticAgent
//!      restricted to {add, subtract} only, sends it a message, and waits.
//!   4. On tick 2, ArithmeticAgent runs: it uses `converse()`, tries `multiply`
//!      (permission denied by capability restriction), falls back to `add`, and
//!      sends the result back to the coordinator.
//!   5. On tick 3, coordinator wakes (message arrived), receives the result,
//!      calls `converse()` to produce the final answer, and finishes.
//!
//! Usage:
//!   cargo run -p calculator_agent
//!
//! The demo runs entirely in-memory with a deterministic MockLlmProvider —
//! no API keys required.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tracing::info;

use tianshu::{
    agent::{Agent, AgentId, CapabilityRestriction},
    agent_context::AgentSpawnConfig,
    agent_workflow::{AgentOutcome, AgentWorkflow, BaseAgent},
    agent_context::AgentContext,
    engine::{SchedulerEnvironment, SchedulerV2},
    llm::{LlmProvider, LlmRequest, LlmResponse, LlmUsage, ToolCall},
    poll::{AgentMessageFetcher, CompositeResourceFetcher},
    session::Session,
    store::{
        AgentStore, CaseStore, InMemoryAgentMessageStore, InMemoryAgentStore, InMemoryCaseStore,
        InMemoryStateStore, StateStore,
    },
    tool::{Tool, ToolRegistry, ToolSafety},
    tool_loop::ToolLoopConfig,
    workflow::{PollPredicate, WorkflowResult},
    WorkflowRegistry,
};

// ── Math Tools ────────────────────────────────────────────────────────────────

struct AddTool;
#[async_trait]
impl Tool for AddTool {
    fn name(&self) -> &str { "add" }
    fn description(&self) -> &str { "Add two numbers: {a, b} -> a + b" }
    fn safety(&self) -> ToolSafety { ToolSafety::ReadOnly }
    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            },
            "required": ["a", "b"]
        })
    }
    async fn execute(&self, input: JsonValue) -> Result<String> {
        let a = input["a"].as_f64().unwrap_or(0.0);
        let b = input["b"].as_f64().unwrap_or(0.0);
        let result = a + b;
        info!("AddTool: {} + {} = {}", a, b, result);
        Ok(result.to_string())
    }
}

struct SubtractTool;
#[async_trait]
impl Tool for SubtractTool {
    fn name(&self) -> &str { "subtract" }
    fn description(&self) -> &str { "Subtract two numbers: {a, b} -> a - b" }
    fn safety(&self) -> ToolSafety { ToolSafety::ReadOnly }
    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            },
            "required": ["a", "b"]
        })
    }
    async fn execute(&self, input: JsonValue) -> Result<String> {
        let a = input["a"].as_f64().unwrap_or(0.0);
        let b = input["b"].as_f64().unwrap_or(0.0);
        let result = a - b;
        info!("SubtractTool: {} - {} = {}", a, b, result);
        Ok(result.to_string())
    }
}

struct MultiplyTool;
#[async_trait]
impl Tool for MultiplyTool {
    fn name(&self) -> &str { "multiply" }
    fn description(&self) -> &str { "Multiply two numbers: {a, b} -> a * b" }
    fn safety(&self) -> ToolSafety { ToolSafety::ReadOnly }
    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            },
            "required": ["a", "b"]
        })
    }
    async fn execute(&self, input: JsonValue) -> Result<String> {
        let a = input["a"].as_f64().unwrap_or(0.0);
        let b = input["b"].as_f64().unwrap_or(0.0);
        let result = a * b;
        info!("MultiplyTool: {} * {} = {}", a, b, result);
        Ok(result.to_string())
    }
}

struct DivideTool;
#[async_trait]
impl Tool for DivideTool {
    fn name(&self) -> &str { "divide" }
    fn description(&self) -> &str { "Divide two numbers: {a, b} -> a / b" }
    fn safety(&self) -> ToolSafety { ToolSafety::ReadOnly }
    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            },
            "required": ["a", "b"]
        })
    }
    async fn execute(&self, input: JsonValue) -> Result<String> {
        let a = input["a"].as_f64().unwrap_or(0.0);
        let b = input["b"].as_f64().unwrap_or(0.0);
        if b == 0.0 {
            return Err(anyhow::anyhow!("division by zero"));
        }
        let result = a / b;
        info!("DivideTool: {} / {} = {}", a, b, result);
        Ok(result.to_string())
    }
}

// ── MockLlmProvider ───────────────────────────────────────────────────────────
//
// Deterministic mock that branches on the conversation context rather than call
// count. Coordinator: multiply → final answer. Arithmetic (restricted): tries
// multiply (gets denied), falls back to add → sends result.

struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let system = request.system_prompt.as_deref().unwrap_or("");
        let last_msg = request.messages.last();
        let last_role = last_msg.map(|m| m.role.as_str()).unwrap_or("");
        let last_content = last_msg.map(|m| m.content.as_str()).unwrap_or("");

        // ── Coordinator logic ─────────────────────────────────────────────────
        if system.contains("coordinator") {
            // Turn 1: user message with math problem → use multiply tool
            if last_role == "user" && last_content.contains("calculate") {
                return Ok(tool_call_response("multiply", r#"{"a":5,"b":2}"#));
            }
            // Turn 2: tool result from multiply → final answer
            if last_role == "tool" {
                let product: f64 = last_content.trim().parse().unwrap_or(10.0);
                let total = 3.0 + product;
                return Ok(text_response(&format!(
                    "The result of 3 + 5 * 2 is {} (multiply first, then add).",
                    total
                )));
            }
            // Tick 3 synthesis: arithmetic agent sent back its result.
            // The user message contains the sub-result and asks for the final answer.
            if last_role == "user" && last_content.contains("arithmetic agent computed") {
                return Ok(text_response(
                    "3 + 5 × 2 = 13 (arithmetic agent confirmed 5×2=10 using add; coordinator adds 3)."
                ));
            }
            // Fallback
            return Ok(text_response("Calculation complete."));
        }

        // ── Arithmetic agent logic ────────────────────────────────────────────
        if system.contains("arithmetic") {
            // Turn 1: user message with sub-problem → try multiply (will be denied)
            if last_role == "user" {
                return Ok(tool_call_response("multiply", r#"{"a":5,"b":2}"#));
            }
            // Turn 2: permission denied on multiply → fall back to add
            if last_role == "tool" && last_content.contains("permission denied") {
                // Simulate 5 * 2 using repeated addition: 5+5+5+5+5 is too complex,
                // so just do 5+5 = 10 to keep the demo simple.
                info!("[arithmetic mock] multiply denied by capabilities — falling back to add");
                return Ok(tool_call_response("add", r#"{"a":5,"b":5}"#));
            }
            // Turn 3: add result arrived → produce final text
            if last_role == "tool" {
                let sum: f64 = last_content.trim().parse().unwrap_or(10.0);
                return Ok(text_response(&format!(
                    "Arithmetic result: {}",
                    sum
                )));
            }
            return Ok(text_response("Done."));
        }

        Ok(text_response("No matching LLM branch."))
    }
}

fn tool_call_response(tool_name: &str, args: &str) -> LlmResponse {
    LlmResponse {
        content: String::new(),
        finish_reason: "tool_calls".to_string(),
        usage: LlmUsage { prompt_tokens: 10, completion_tokens: 10 },
        tool_calls: Some(vec![ToolCall {
            id: format!("call_{}", tool_name),
            name: tool_name.to_string(),
            arguments: args.to_string(),
        }]),
    }
}

fn text_response(text: &str) -> LlmResponse {
    LlmResponse {
        content: text.to_string(),
        finish_reason: "stop".to_string(),
        usage: LlmUsage { prompt_tokens: 10, completion_tokens: 10 },
        tool_calls: None,
    }
}

// ── CoordinatorAgent ──────────────────────────────────────────────────────────

struct CoordinatorAgent {
    llm: Arc<MockLlmProvider>,
}

#[async_trait]
impl BaseAgent for CoordinatorAgent {
    async fn act(&self, ctx: &mut AgentContext<'_>) -> Result<AgentOutcome> {
        // Persist state across ticks with a simple string key.
        let phase: String = ctx.get_state("phase", "init".to_string()).await?;
        let coordinator_id = ctx.agent_id().clone();

        match phase.as_str() {
            // ── Tick 1: solve the coordinator's portion, spawn arithmetic child ──
            "init" => {
                let problem = ctx
                    .case()
                    .resource_data
                    .as_ref()
                    .and_then(|d| d["problem"].as_str())
                    .unwrap_or("calculate 3 + 5 * 2")
                    .to_string();

                info!("[coordinator] Received problem: {}", problem);

                // Use converse() to produce a tool call (multiply) and get its result.
                // The MockLlmProvider handles the full tool loop in memory.
                let config = ToolLoopConfig::default();
                let answer = ctx
                    .converse(
                        &format!("calculate {}", problem),
                        self.llm.as_ref(),
                        "mock-model",
                        Some("You are the coordinator agent."),
                        &config,
                    )
                    .await?;

                info!("[coordinator] Own calculation result: {}", answer);

                // Spawn an ArithmeticAgent with restricted tools (add + subtract only).
                // This demonstrates capability narrowing — multiply is intentionally denied.
                let arithmetic_id = "arithmetic_agent_001";
                let handle = ctx
                    .spawn_agent(AgentSpawnConfig {
                        agent_id: arithmetic_id.to_string(),
                        role: "arithmetic".to_string(),
                        workflow_code: "arithmetic".to_string(),
                        restriction: Some(CapabilityRestriction {
                            allowed_tools: Some(vec![
                                "add".to_string(),
                                "subtract".to_string(),
                            ]),
                            can_spawn: Some(false),
                            max_spawn_depth: None,
                            can_send_messages: Some(true),
                            visible_session_keys: None,
                            writable_session_keys: None,
                        }),
                        config: None,
                        resource_data: Some(serde_json::json!({
                            "problem": "compute 5 * 2 using only add and subtract"
                        })),
                    })
                    .await?;

                info!(
                    "[coordinator] Spawned ArithmeticAgent: {}",
                    handle.agent_id
                );

                // Send the sub-problem to the arithmetic agent.
                ctx.send_message(
                    &handle.agent_id,
                    serde_json::json!({
                        "from": "coordinator",
                        "task": "compute 5 * 2 using only add/subtract",
                        "reply_to": coordinator_id.as_str()
                    }),
                )
                .await?;

                info!("[coordinator] Sent task to arithmetic agent, now waiting");

                // Persist handle agent_id so we can reference it next tick.
                ctx.set_state("child_agent_id", arithmetic_id.to_string()).await?;
                ctx.set_state("phase", "waiting_child".to_string()).await?;

                // Return Waiting — poll for a message addressed to coordinator.
                Ok(WorkflowResult::Waiting(vec![PollPredicate {
                    resource_type: "agent_message".to_string(),
                    resource_id: coordinator_id.as_str().to_string(),
                    step_name: "wait_for_arithmetic_result".to_string(),
                    intent_desc: None,
                }]))
            }

            // ── Tick 3: arithmetic result arrived ─────────────────────────────
            "waiting_child" => {
                let messages = ctx.receive_messages().await?;
                if messages.is_empty() {
                    info!("[coordinator] No messages yet, still waiting");
                    return Ok(WorkflowResult::Waiting(vec![PollPredicate {
                        resource_type: "agent_message".to_string(),
                        resource_id: coordinator_id.as_str().to_string(),
                        step_name: "wait_for_arithmetic_result".to_string(),
                        intent_desc: None,
                    }]));
                }

                let msg = &messages[0];
                let sub_result = msg.payload["result"].as_str().unwrap_or("10").to_string();
                info!("[coordinator] Received arithmetic result: {}", sub_result);

                // Acknowledge so they're marked consumed.
                let ids: Vec<String> = messages.iter().map(|m| m.message_id.clone()).collect();
                ctx.acknowledge_messages(&ids).await?;

                // Use converse() for a final synthesis.
                let config = ToolLoopConfig::default();
                let final_answer = ctx
                    .converse(
                        &format!(
                            "The arithmetic agent computed 5*2={} (using add). What is 3 + 5*2?",
                            sub_result
                        ),
                        self.llm.as_ref(),
                        "mock-model",
                        Some("You are the coordinator agent."),
                        &config,
                    )
                    .await?;

                info!("[coordinator] Final answer: {}", final_answer);
                ctx.set_state("phase", "done".to_string()).await?;
                ctx.finish("SUCCESS".to_string(), final_answer).await?;
                Ok(WorkflowResult::Finished(
                    "SUCCESS".to_string(),
                    "Calculation complete".to_string(),
                ))
            }

            other => {
                info!("[coordinator] Unknown phase: {}", other);
                ctx.finish("ERROR".to_string(), format!("unknown phase: {}", other))
                    .await?;
                Ok(WorkflowResult::Finished(
                    "ERROR".to_string(),
                    format!("unknown phase: {}", other),
                ))
            }
        }
    }
}

// ── ArithmeticAgent ───────────────────────────────────────────────────────────

struct ArithmeticAgent {
    llm: Arc<MockLlmProvider>,
}

#[async_trait]
impl BaseAgent for ArithmeticAgent {
    async fn act(&self, ctx: &mut AgentContext<'_>) -> Result<AgentOutcome> {
        let agent_id = ctx.agent_id().clone();

        // Receive the task message from coordinator.
        let messages = ctx.receive_messages().await?;
        if messages.is_empty() {
            info!("[arithmetic] No message yet, returning Continue to be re-run next tick");
            return Ok(WorkflowResult::Continue);
        }

        let msg = &messages[0];
        let task = msg.payload["task"].as_str().unwrap_or("compute 5*2").to_string();
        let reply_to_str = msg.payload["reply_to"].as_str().unwrap_or("").to_string();
        let reply_to = AgentId::new(&reply_to_str);

        info!("[arithmetic] Got task: {}", task);

        // Acknowledge the message.
        ctx.acknowledge_messages(&[msg.message_id.clone()]).await?;

        // Use converse() with scoped tools (add + subtract only).
        // The mock will attempt multiply (denied → permission error), then fall back to add.
        let config = ToolLoopConfig::default();
        let result = ctx
            .converse(
                &task,
                self.llm.as_ref(),
                "mock-model",
                Some("You are the arithmetic agent. Use available tools only."),
                &config,
            )
            .await?;

        info!("[arithmetic] Computed result text: {}", result);

        // Extract the numeric answer from the final text.
        // The MockLlmProvider returns "Arithmetic result: 10".
        let numeric_result = result
            .split_whitespace()
            .last()
            .unwrap_or("10")
            .trim_end_matches('.')
            .to_string();

        info!(
            "[arithmetic] Sending result '{}' back to coordinator '{}'",
            numeric_result, reply_to
        );

        // Send result back to coordinator.
        ctx.send_message(
            &reply_to,
            serde_json::json!({
                "from": agent_id.as_str(),
                "result": numeric_result
            }),
        )
        .await?;

        ctx.finish("SUCCESS".to_string(), format!("result={}", numeric_result))
            .await?;
        Ok(WorkflowResult::Finished(
            "SUCCESS".to_string(),
            format!("arithmetic result={}", numeric_result),
        ))
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    info!("=== Calculator Agent Demo — Phase 4 Agent Architecture ===");

    // ── Shared stores ─────────────────────────────────────────────────────────
    let case_store: Arc<dyn CaseStore> = Arc::new(InMemoryCaseStore::default());
    let state_store: Arc<dyn StateStore> = Arc::new(InMemoryStateStore::default());
    let agent_store: Arc<dyn AgentStore> = Arc::new(InMemoryAgentStore::default());
    let message_store = Arc::new(InMemoryAgentMessageStore::default());

    // ── Tools ─────────────────────────────────────────────────────────────────
    // All four tools go into the global registry.
    // ScopedToolRegistry (inside AgentContext) filters them per-agent by Capabilities.
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(AddTool);
    tool_registry.register(SubtractTool);
    tool_registry.register(MultiplyTool);
    tool_registry.register(DivideTool);
    let tool_registry = Arc::new(tool_registry);

    let llm = Arc::new(MockLlmProvider);

    // ── Register workflows ────────────────────────────────────────────────────
    let mut registry = WorkflowRegistry::new();

    {
        let llm_c = llm.clone();
        AgentWorkflow::register(
            &mut registry,
            "coordinator",
            tool_registry.clone(),
            agent_store.clone(),
            message_store.clone(),
            move |_agent| {
                Box::new(CoordinatorAgent {
                    llm: llm_c.clone(),
                })
            },
        );
    }

    {
        let llm_a = llm.clone();
        AgentWorkflow::register(
            &mut registry,
            "arithmetic",
            tool_registry.clone(),
            agent_store.clone(),
            message_store.clone(),
            move |_agent| {
                Box::new(ArithmeticAgent {
                    llm: llm_a.clone(),
                })
            },
        );
    }

    // ── Create root agent + backing case ──────────────────────────────────────
    let coordinator_id = "coordinator_agent_001";
    let (root_agent, mut root_case) = Agent::root(
        coordinator_id,
        "coordinator",
        "calc_session",
        "coordinator",
    );

    // Embed the problem in the case's resource_data.
    root_case.resource_data = Some(serde_json::json!({
        "problem": "3 + 5 * 2"
    }));

    // Persist root agent and case.
    agent_store.upsert(&root_agent).await?;
    case_store.upsert(&root_case).await?;

    // ── Scheduler environment ─────────────────────────────────────────────────
    let session = Session::new("calc_session");
    let mut env = SchedulerEnvironment::new(session, vec![root_case]);
    let mut scheduler = SchedulerV2::new();

    // ── ResourceFetcher: AgentMessageFetcher resolves "agent_message" polls ──
    let message_fetcher = Arc::new(AgentMessageFetcher::new(message_store.clone()));
    let fetcher = CompositeResourceFetcher::new(vec![message_fetcher]);

    // ── Tick 1: Coordinator runs, uses converse(), spawns arithmetic agent ────
    info!("--- Tick 1: Coordinator initialises ---");
    let result = scheduler
        .tick(
            &mut env,
            &registry,
            case_store.clone(),
            state_store.clone(),
            Some(&fetcher),
            None,
        )
        .await?;
    info!("Tick 1 result: {:?}", result);

    // spawn_child() only writes to CaseStore — refresh env so the scheduler
    // can see the new arithmetic case on subsequent ticks.
    let all_cases = case_store.get_by_session("calc_session").await?;
    for c in all_cases {
        env.current_case_dict
            .entry(c.case_key.clone())
            .or_insert(c);
    }
    info!(
        "Cases in env after tick 1: {:?}",
        env.current_case_dict.keys().collect::<Vec<_>>()
    );

    // ── Tick 2: ArithmeticAgent runs, tries multiply (denied), uses add ───────
    info!("--- Tick 2: Arithmetic agent runs ---");
    let result = scheduler
        .tick(
            &mut env,
            &registry,
            case_store.clone(),
            state_store.clone(),
            Some(&fetcher),
            None,
        )
        .await?;
    info!("Tick 2 result: {:?}", result);

    // Sync finished arithmetic case back into env (coordinator checks its status).
    let all_cases = case_store.get_by_session("calc_session").await?;
    for c in all_cases {
        env.current_case_dict.insert(c.case_key.clone(), c);
    }

    // ── Tick 3: Coordinator wakes, receives message, produces final answer ─────
    info!("--- Tick 3: Coordinator wakes and completes ---");
    let result = scheduler
        .tick(
            &mut env,
            &registry,
            case_store.clone(),
            state_store.clone(),
            Some(&fetcher),
            None,
        )
        .await?;
    info!("Tick 3 result: {:?}", result);

    // ── Summary ───────────────────────────────────────────────────────────────
    info!("=== Final case states ===");
    for (key, case) in &env.current_case_dict {
        info!(
            "  {}: state={:?}, finished_type={:?}, desc={:?}",
            key, case.execution_state, case.finished_type, case.finished_description
        );
    }

    let coord = &env.current_case_dict[coordinator_id];
    assert_eq!(
        coord.finished_type.as_deref(),
        Some("SUCCESS"),
        "Coordinator should finish with SUCCESS"
    );
    info!("=== Demo complete ===");

    Ok(())
}
