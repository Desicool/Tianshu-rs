//! Multi-turn conversational agent workflow.
//!
//! Each tick waits for a `user_message` in `case.resource_data`, sends it to
//! the LLM (with the full conversation history), and appends both the user
//! message and the assistant reply to the persisted history.  The workflow then
//! returns `WorkflowResult::Waiting` so the next user input can be delivered.
//!
//! Checkpointing: each turn is wrapped in `ctx.step("turn_{n}")`, giving
//! idempotency across crashes.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tracing::info;

use example_tools::{run_tool_loop, ToolRegistry};
use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, LlmProvider, LlmRequest},
    workflow::{BaseWorkflow, WorkflowResult},
};

// ── ConversationWorkflow ──────────────────────────────────────────────────────

pub struct ConversationWorkflow {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
    pub system_prompt: String,
}

#[async_trait]
impl BaseWorkflow for ConversationWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        // 1. Load message history.
        let mut history: Vec<LlmMessage> = ctx
            .get_state("conv_messages", Vec::<LlmMessage>::new())
            .await?;

        // 2. Check for new user input.
        let user_message: Option<String> = ctx
            .case
            .resource_data
            .as_ref()
            .and_then(|d| d["user_message"].as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let user_message = match user_message {
            Some(msg) => msg,
            None => return Ok(WorkflowResult::Waiting(vec![])),
        };

        // 3. Avoid re-processing the same message when the REPL re-ticks without
        //    delivering new input (e.g. the engine probes the Waiting case and
        //    resource_data still holds the previous message).
        let last_input: String = ctx.get_state("conv_last_input", String::new()).await?;
        if last_input == user_message {
            return Ok(WorkflowResult::Waiting(vec![]));
        }

        // 4. Append user message to history.
        history.push(LlmMessage::user(user_message.clone()));

        // 5. Determine current turn number.
        let turn_n: u64 = ctx.get_state("conv_turn", 0u64).await?;

        // 6. Capture values for the move closure.
        let llm = Arc::clone(&self.llm);
        let model = self.model.clone();
        let tools = Arc::clone(&self.tools);
        let system_prompt = self.system_prompt.clone();
        let history_snapshot = history.clone();
        let initial_len = history_snapshot.len();

        // 7. Run the LLM turn inside a checkpointed step.
        // The step returns a JsonValue containing { text, new_messages }.
        let step_name = format!("turn_{turn_n}");
        let payload: JsonValue = ctx
            .step(&step_name, move |_ctx| async move {
                let request = LlmRequest {
                    model,
                    system_prompt: Some(system_prompt),
                    messages: history_snapshot,
                    temperature: None,
                    max_tokens: None,
                    tools: Some(tools.to_llm_tools()),
                };
                // run_tool_loop returns (final_text, all_request_messages_at_return).
                // `all_messages` includes the initial messages + any tool-call/tool-result
                // messages appended during the loop.  The final stop assistant message
                // is NOT appended to request.messages inside run_tool_loop — it is
                // only returned as `final_text`.
                let (text, all_messages) = run_tool_loop(llm.as_ref(), request, &tools, 3).await?;

                // New messages = those appended during the tool loop (tool calls +
                // tool results only; final assistant reply is in `text`).
                let new_messages = all_messages[initial_len..].to_vec();

                let result = serde_json::json!({
                    "text": text,
                    "new_messages": serde_json::to_value(&new_messages)?
                });
                Ok(result)
            })
            .await?;

        // 8. Extract final text and new intermediate messages from step payload.
        let final_text = payload["text"].as_str().unwrap_or("").to_string();
        let new_messages: Vec<LlmMessage> =
            serde_json::from_value(payload["new_messages"].clone())?;

        // 9. Append new intermediate messages (tool calls / tool results) to history.
        for msg in new_messages {
            history.push(msg);
        }

        // 10. Append the final assistant reply if present and not already there.
        if !final_text.is_empty() {
            let already_there = history
                .last()
                .map(|m| m.role == "assistant" && m.content == final_text)
                .unwrap_or(false);
            if !already_there {
                history.push(LlmMessage::assistant(final_text));
            }
        }

        // 11. Persist updated state.
        ctx.set_state("conv_messages", &history).await?;
        ctx.set_state("conv_turn", turn_n + 1).await?;
        ctx.set_state("conv_last_input", user_message).await?;

        info!(
            "ConversationWorkflow: case_key={}, turn={}, history_len={}",
            ctx.case.case_key,
            turn_n,
            history.len()
        );

        Ok(WorkflowResult::Waiting(vec![]))
    }
}
