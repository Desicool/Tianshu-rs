// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

/// `StreamingLlmProvider` implementation for OpenAI-compatible APIs.
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::debug;

use tianshu::llm::{LlmRequest, LlmStreamEvent, LlmUsage, StreamingLlmProvider, ToolCall};

use crate::sse::{parse_sse_chunk, SseChunk};
use crate::OpenAiProvider;

#[async_trait]
impl StreamingLlmProvider for OpenAiProvider {
    /// Stream a completion via SSE.
    ///
    /// Sends `LlmStreamEvent` values through `tx` as they arrive.
    /// Always sends `Done` or `Error` as the final event.
    async fn stream(&self, request: LlmRequest, tx: mpsc::Sender<LlmStreamEvent>) -> Result<()> {
        let url = format!("{}/chat/completions", self.base_url());

        // Build wire request body (re-use JSON serialisation)
        let mut body = serde_json::json!({
            "model": request.model,
            "stream": true,
            "messages": build_wire_messages(&request),
        });

        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(max) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max);
        }
        if let Some(tools) = &request.tools {
            body["tools"] = serde_json::to_value(tools)?;
        }

        debug!(
            "OpenAI streaming request: model={}, url={}",
            request.model, url
        );

        let response = self
            .client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key()))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            let text = response.text().await.unwrap_or_default();
            let err_msg = format!("LLM API HTTP {}: {}", status.as_u16(), text);
            let _ = tx.send(LlmStreamEvent::Error(err_msg.clone())).await;
            return Err(anyhow::anyhow!(err_msg));
        }

        let mut stream = response.bytes_stream();
        let mut buf = String::new();

        // Accumulators for tool calls
        let mut tool_call_acc: std::collections::HashMap<usize, (String, String, String)> =
            std::collections::HashMap::new();

        while let Some(bytes_result) = stream.next().await {
            let bytes = match bytes_result {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(LlmStreamEvent::Error(e.to_string())).await;
                    return Err(e.into());
                }
            };

            buf.push_str(&String::from_utf8_lossy(&bytes));

            // Process complete lines
            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim().to_string();
                buf = buf[newline_pos + 1..].to_string();

                match parse_sse_chunk(&line) {
                    None => {}
                    Some(SseChunk::TextDelta(s)) if !s.is_empty() => {
                        if tx.send(LlmStreamEvent::TextDelta(s)).await.is_err() {
                            return Ok(()); // receiver dropped
                        }
                    }
                    Some(SseChunk::TextDelta(_)) => {}
                    Some(SseChunk::ToolCallStart { id, name }) => {
                        tool_call_acc.entry(0).or_insert((id, name, String::new()));
                    }
                    Some(SseChunk::ToolCallArgsDelta { index, args_delta }) => {
                        if let Some(entry) = tool_call_acc.get_mut(&index) {
                            entry.2.push_str(&args_delta);
                        }
                    }
                    Some(SseChunk::Usage {
                        prompt_tokens,
                        completion_tokens,
                    }) => {
                        let _ = tx
                            .send(LlmStreamEvent::Usage(LlmUsage {
                                prompt_tokens,
                                completion_tokens,
                            }))
                            .await;
                    }
                    Some(SseChunk::FinishReason(_)) => {}
                    Some(SseChunk::Done) => {
                        // Emit accumulated tool calls
                        let mut indices: Vec<usize> = tool_call_acc.keys().cloned().collect();
                        indices.sort();
                        for idx in indices {
                            if let Some((id, name, args)) = tool_call_acc.remove(&idx) {
                                let _ = tx
                                    .send(LlmStreamEvent::ToolUse(ToolCall {
                                        id,
                                        name,
                                        arguments: args,
                                    }))
                                    .await;
                            }
                        }
                        let _ = tx.send(LlmStreamEvent::Done("stop".into())).await;
                        return Ok(());
                    }
                }
            }
        }

        // Stream ended without [DONE] — emit any pending tool calls and close
        let mut indices: Vec<usize> = tool_call_acc.keys().cloned().collect();
        indices.sort();
        for idx in indices {
            if let Some((id, name, args)) = tool_call_acc.remove(&idx) {
                let _ = tx
                    .send(LlmStreamEvent::ToolUse(ToolCall {
                        id,
                        name,
                        arguments: args,
                    }))
                    .await;
            }
        }
        let _ = tx.send(LlmStreamEvent::Done("stop".into())).await;
        Ok(())
    }
}

/// Build the `messages` array for the wire request, including system prompt.
fn build_wire_messages(request: &LlmRequest) -> serde_json::Value {
    let mut msgs = Vec::new();

    if let Some(sys) = &request.system_prompt {
        msgs.push(serde_json::json!({ "role": "system", "content": sys }));
    }

    for msg in &request.messages {
        let mut m = serde_json::json!({
            "role": msg.role,
            "content": msg.content,
        });
        if let Some(tc) = &msg.tool_calls {
            m["tool_calls"] = serde_json::to_value(tc).unwrap_or_default();
        }
        if let Some(id) = &msg.tool_call_id {
            m["tool_call_id"] = serde_json::json!(id);
        }
        msgs.push(m);
    }

    serde_json::json!(msgs)
}
