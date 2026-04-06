//! Synthesize stage: produce the final answer from the accumulated conversation.
//!
//! The last assistant message in `orch_messages` already contains the LLM's
//! final text (set by PlanTools when it got a "stop" response).  We extract it
//! and return `StageOutcome::finish`.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tracing::info;

use workflow_engine::{
    context::WorkflowContext,
    llm::{LlmMessage, LlmProvider, LlmRequest},
    stage::{StageBase, StageOutcome},
};

use crate::workflows::OrchStage;

pub struct SynthesizeStage {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
}

#[async_trait]
impl StageBase<OrchStage> for SynthesizeStage {
    fn stage_key(&self) -> OrchStage {
        OrchStage::Synthesize
    }

    fn step_keys(&self) -> &[&str] {
        &["synthesize_step"]
    }

    async fn execute(&self, ctx: &mut WorkflowContext) -> Result<StageOutcome<OrchStage>> {
        let messages: Vec<LlmMessage> = ctx
            .get_state("orch_messages", Vec::<LlmMessage>::new())
            .await?;

        // The last message is the assistant's final answer appended by PlanTools.
        // We use it directly without an extra LLM call to avoid redundancy.
        // If there is no assistant message, make one final synthesis call.
        let final_answer = messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.content.clone())
            .filter(|c| !c.is_empty());

        let answer = if let Some(text) = final_answer {
            info!("Synthesize: using existing assistant message");
            text
        } else {
            // Fallback: make one more LLM call without tools to generate a summary.
            info!("Synthesize: no usable assistant message, calling LLM for synthesis");
            let request = LlmRequest {
                model: self.model.clone(),
                system_prompt: Some(
                    "Summarise the conversation and provide a clear final answer.".to_string(),
                ),
                messages: messages.clone(),
                temperature: None,
                max_tokens: None,
                tools: None,
            };
            let response = self.llm.complete(request).await?;
            if response.content.is_empty() {
                return Err(anyhow!("Synthesize: LLM returned empty content"));
            }
            response.content
        };

        info!("Synthesize: final answer ({} chars)", answer.len());

        Ok(StageOutcome::finish("completed", answer))
    }
}
