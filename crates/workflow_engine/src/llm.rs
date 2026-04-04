use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A single message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

/// Request sent to an LLM provider.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub system_prompt: Option<String>,
    pub messages: Vec<LlmMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
}

/// Token usage reported by the LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// Response from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub usage: LlmUsage,
    pub finish_reason: String,
}

/// Abstraction over any LLM provider.
///
/// Implement this trait to plug in OpenAI, Claude, Doubao, Ollama, or any
/// other provider. The `workflow-engine-llm-openai` crate ships a ready-made
/// implementation for OpenAI-compatible APIs.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;
}
