// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
/// Tests for LlmProvider trait and LlmRequest/LlmResponse types.
use workflow_engine::llm::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmUsage};

struct EchoLlm;

#[async_trait]
impl LlmProvider for EchoLlm {
    async fn complete(&self, request: LlmRequest) -> anyhow::Result<LlmResponse> {
        // Echo back the last user message content
        let content = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(LlmResponse {
            content,
            usage: LlmUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            },
            finish_reason: "stop".into(),

            tool_calls: None,
        })
    }
}

#[tokio::test]
async fn llm_provider_echo_impl() {
    let llm = EchoLlm;
    let req = LlmRequest {
        model: "test".into(),
        system_prompt: None,
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "hello world".into(),

            tool_calls: None,

            tool_call_id: None,
        }],
        temperature: Some(0.0),
        max_tokens: Some(64),

        tools: None,
    };

    let resp = llm.complete(req).await.unwrap();
    assert_eq!(resp.content, "hello world");
    assert_eq!(resp.finish_reason, "stop");
    assert_eq!(resp.usage.prompt_tokens, 10);
}

#[tokio::test]
async fn llm_request_with_system_prompt() {
    let llm = EchoLlm;
    let req = LlmRequest {
        model: "gpt-4".into(),
        system_prompt: Some("You are a helpful assistant".into()),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "ping".into(),

            tool_calls: None,

            tool_call_id: None,
        }],
        temperature: None,
        max_tokens: None,

        tools: None,
    };

    let resp = llm.complete(req).await.unwrap();
    assert_eq!(resp.content, "ping");
}