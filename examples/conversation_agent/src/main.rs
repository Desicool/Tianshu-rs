//! Conversational agent CLI REPL.
//!
//! Reads lines from stdin and sends them to the conversation workflow.
//! Type "quit" or "exit" to stop.
//!
//! Environment variables:
//!   OPENAI_API_KEY   — required
//!   OPENAI_MODEL     — model name (default: "gpt-4o-mini")
//!   OPENAI_BASE_URL  — API base URL (default: OpenAI)
use std::io::{self, Write};
use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use workflow_engine::{
    case::Case,
    engine::{SchedulerEnvironment, SchedulerV2, TickResult},
    llm::LlmMessage,
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
    WorkflowRegistry,
};
use workflow_engine_llm_openai::OpenAiProvider;

use conversation_agent::register_workflows;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("warn").init();

    // ── Configuration from environment ───────────────────────────────────────
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-placeholder".to_string());
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let base_url = std::env::var("OPENAI_BASE_URL").ok();

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
    let state_store = Arc::new(InMemoryStateStore::default());
    let state_store_dyn: Arc<dyn StateStore> = Arc::clone(&state_store) as Arc<dyn StateStore>;

    let mut registry = WorkflowRegistry::new();
    register_workflows(&mut registry, Arc::clone(&llm), model.clone());

    // ── Case setup ────────────────────────────────────────────────────────────
    let case_key = "conversation_repl_001";
    let case = Case::new(
        case_key.into(),
        "repl_session".into(),
        "conversation".into(),
    );

    let mut env = SchedulerEnvironment::from_session_id("repl_session", vec![case]);
    let mut scheduler = SchedulerV2::new();

    println!("Conversational Agent (type 'quit' or 'exit' to stop)");
    println!("------------------------------------------------------");

    // ── REPL loop ─────────────────────────────────────────────────────────────
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim().to_string();

        if input.is_empty() {
            continue;
        }
        if input == "quit" || input == "exit" {
            println!("Goodbye!");
            break;
        }

        // Set the user message on the case.
        let conv_case = env
            .current_case_dict
            .get_mut(case_key)
            .expect("case should exist");
        conv_case.resource_data = Some(serde_json::json!({ "user_message": input }));

        // Tick until the workflow returns Waiting.
        loop {
            let result = scheduler
                .tick(
                    &mut env,
                    &registry,
                    Arc::clone(&case_store),
                    Arc::clone(&state_store_dyn),
                    None,
                    None,
                )
                .await?;

            match result {
                TickResult::Idle => break,
                TickResult::Executed => {}
                TickResult::ShutdownRequested => break,
            }

            // Check if workflow finished (shouldn't normally happen in REPL mode).
            use workflow_engine::case::ExecutionState;
            if env.current_case_dict[case_key].execution_state == ExecutionState::Finished {
                break;
            }
        }

        // Read the latest assistant message from state.
        if let Ok(Some(entry)) = state_store.get(case_key, "wf_state_conv_messages").await {
            let messages: Vec<LlmMessage> = serde_json::from_str(&entry.data).unwrap_or_default();
            if let Some(last_asst) = messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.is_empty())
        {
                println!("Agent: {}", last_asst.content);
            }
        }
    }

    Ok(())
}
