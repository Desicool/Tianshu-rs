// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;

// ── Tool types ────────────────────────────────────────────────────────────────

/// A tool that can be offered to an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    /// JSON Schema describing the tool's parameters.
    pub parameters: JsonValue,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this call (used to correlate with results).
    pub id: String,
    pub name: String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}

/// The result of executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Must match the `id` from the originating `ToolCall`.
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

// ── Streaming types ───────────────────────────────────────────────────────────

/// Events emitted by a streaming LLM response.
#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    /// Incremental text content.
    TextDelta(String),
    /// A complete tool call has been parsed from the stream.
    ToolUse(ToolCall),
    /// Token usage (may arrive mid-stream or at the end).
    Usage(LlmUsage),
    /// The stream has ended; contains the finish reason.
    Done(String),
    /// An error occurred during streaming.
    Error(String),
}

// ── Message ───────────────────────────────────────────────────────────────────

/// A single message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
    /// Tool calls requested by an assistant message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// For tool-result messages: the call ID this result responds to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Request sent to an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub system_prompt: Option<String>,
    pub messages: Vec<LlmMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    /// Tools available to the model for this request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<LlmTool>>,
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
    /// Tool calls requested by the model (when `finish_reason` is "tool_calls").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
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

/// Extension trait for LLM providers that support token-by-token streaming.
///
/// Streaming providers send events through an `mpsc::Sender` rather than
/// returning a completed response, enabling parallel tool execution as
/// tool calls arrive from the stream.
#[async_trait]
pub trait StreamingLlmProvider: LlmProvider {
    /// Stream a completion. Events are sent through `tx` as they arrive.
    ///
    /// Completes (returns `Ok`) after sending `LlmStreamEvent::Done` or
    /// `LlmStreamEvent::Error`. The caller owns the receiver end.
    async fn stream(&self, request: LlmRequest, tx: mpsc::Sender<LlmStreamEvent>) -> Result<()>;
}
