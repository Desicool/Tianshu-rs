// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

/// Tests for OpenAI-compatible LlmProvider adapter.
///
/// These tests verify construction, configuration, and the LlmProvider
/// trait contract WITHOUT making real HTTP calls.
use std::sync::Arc;
use tianshu::llm::{LlmMessage, LlmProvider, LlmRequest};
use tianshu_llm_openai::OpenAiProvider;

#[test]
fn provider_default_base_url() {
    let p = OpenAiProvider::new("sk-test-key", "gpt-4o");
    assert_eq!(p.base_url(), "https://api.openai.com/v1");
}

#[test]
fn provider_custom_base_url() {
    let p = OpenAiProvider::builder("sk-test", "my-model")
        .base_url("http://localhost:11434/v1")
        .build();
    assert_eq!(p.base_url(), "http://localhost:11434/v1");
}

#[test]
fn provider_strips_trailing_slash_from_base_url() {
    let p = OpenAiProvider::builder("key", "model")
        .base_url("https://api.openai.com/v1/")
        .build();
    assert_eq!(p.base_url(), "https://api.openai.com/v1");
}

#[test]
fn provider_model_name_accessible() {
    let p = OpenAiProvider::new("k", "claude-3-haiku");
    assert_eq!(p.model(), "claude-3-haiku");
}

#[test]
fn provider_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OpenAiProvider>();
}

#[test]
fn provider_implements_llm_provider_trait() {
    // Verify it can be used as Arc<dyn LlmProvider>
    let p: Arc<dyn LlmProvider> = Arc::new(OpenAiProvider::new("key", "model"));
    let _ = p;
}

#[test]
fn llm_request_construction() {
    let req = LlmRequest {
        model: "gpt-4o".into(),
        system_prompt: Some("You are helpful".into()),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "Hello".into(),

            tool_calls: None,

            tool_call_id: None,
        }],
        temperature: Some(0.7),
        max_tokens: Some(100),

        tools: None,
    };
    assert_eq!(req.model, "gpt-4o");
    assert_eq!(req.messages.len(), 1);
}