use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

use workflow_engine::llm::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmUsage};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const REQUEST_TIMEOUT_SECS: u64 = 120;

// ── Wire types (internal) ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct WireMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct WireChoice {
    message: WireChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireChoiceMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct WireResponse {
    choices: Option<Vec<WireChoice>>,
    usage: Option<WireUsage>,
}

// ── Builder ───────────────────────────────────────────────────────────────────

pub struct OpenAiProviderBuilder {
    api_key: String,
    model: String,
    base_url: String,
    timeout_secs: u64,
}

impl OpenAiProviderBuilder {
    fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_secs: REQUEST_TIMEOUT_SECS,
        }
    }

    /// Override the API base URL.
    ///
    /// Use this to point to:
    /// - Azure OpenAI: `https://<resource>.openai.azure.com/openai/deployments/<deployment>`
    /// - ByteDance Doubao: `https://ark.cn-beijing.volces.com/api/v3`
    /// - Ollama local: `http://localhost:11434/v1`
    /// - Any OpenAI-compatible endpoint
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn build(self) -> OpenAiProvider {
        OpenAiProvider {
            api_key: self.api_key,
            model: self.model,
            base_url: self.base_url.trim_end_matches('/').to_string(),
            client: Client::builder()
                .timeout(Duration::from_secs(self.timeout_secs))
                .build()
                .expect("failed to build reqwest client"),
        }
    }
}

// ── OpenAiProvider ────────────────────────────────────────────────────────────

/// `LlmProvider` implementation for OpenAI-compatible chat completion APIs.
///
/// Works with:
/// - OpenAI (default: `https://api.openai.com/v1`)
/// - Azure OpenAI
/// - ByteDance Doubao (`https://ark.cn-beijing.volces.com/api/v3`)
/// - Ollama (`http://localhost:11434/v1`)
/// - Any endpoint that implements `POST /chat/completions`
pub struct OpenAiProvider {
    api_key: String,
    model: String,
    base_url: String,
    client: Client,
}

impl OpenAiProvider {
    /// Create with default OpenAI base URL.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::builder(api_key, model).build()
    }

    /// Create a builder for full configuration.
    pub fn builder(
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> OpenAiProviderBuilder {
        OpenAiProviderBuilder::new(api_key, model)
    }

    /// The configured base URL (trailing slash stripped).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The configured model name.
    pub fn model(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let url = format!("{}/chat/completions", self.base_url);

        let mut messages: Vec<WireMessage> = Vec::new();

        if let Some(sys) = &request.system_prompt {
            messages.push(WireMessage {
                role: "system".into(),
                content: sys.clone(),
            });
        }

        for msg in &request.messages {
            messages.push(WireMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        let body = WireRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        debug!(
            "OpenAI request: model={}, url={}, msgs={}",
            body.model,
            url,
            body.messages.len()
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("LLM API HTTP {}: {}", status.as_u16(), text));
        }

        let wire: WireResponse = response.json().await?;

        let choices = wire.choices.unwrap_or_default();
        if choices.is_empty() {
            return Err(anyhow!("LLM API returned empty choices"));
        }

        let content = choices[0]
            .message
            .content
            .clone()
            .ok_or_else(|| anyhow!("LLM returned None content"))?;

        let finish_reason = choices[0]
            .finish_reason
            .clone()
            .unwrap_or_else(|| "stop".into());

        let usage = match wire.usage {
            Some(u) => LlmUsage {
                prompt_tokens: u.prompt_tokens.unwrap_or(0),
                completion_tokens: u.completion_tokens.unwrap_or(0),
            },
            None => LlmUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
            },
        };

        info!(
            "OpenAI response: model={}, prompt_tokens={}, completion_tokens={}, finish={}",
            request.model, usage.prompt_tokens, usage.completion_tokens, finish_reason
        );

        Ok(LlmResponse {
            content,
            usage,
            finish_reason,
        })
    }
}

