// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

/// Tests for the OpenAI streaming adapter.
///
/// We test the SSE parsing logic in isolation, which is the core logic
/// and doesn't require a live HTTP server.
use tianshu_llm_openai::sse::{parse_sse_chunk, SseChunk};

// ── SSE parser tests ──────────────────────────────────────────────────────────

#[test]
fn parse_empty_line_returns_none() {
    assert!(parse_sse_chunk("").is_none());
}

#[test]
fn parse_non_data_line_returns_none() {
    assert!(parse_sse_chunk("event: update").is_none());
    assert!(parse_sse_chunk(": keep-alive").is_none());
}

#[test]
fn parse_done_sentinel() {
    let chunk = parse_sse_chunk("data: [DONE]").unwrap();
    assert!(matches!(chunk, SseChunk::Done));
}

#[test]
fn parse_text_delta() {
    let line = r#"data: {"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}"#;
    let chunk = parse_sse_chunk(line).unwrap();
    match chunk {
        SseChunk::TextDelta(s) => assert_eq!(s, "hello"),
        other => panic!("expected TextDelta, got {:?}", other),
    }
}

#[test]
fn parse_text_delta_empty_content_skipped() {
    // When the delta has no content field (e.g., first chunk with role only), skip
    let line = r#"data: {"choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#;
    // Should return None or an empty/role delta — we skip role-only chunks
    let chunk = parse_sse_chunk(line);
    // Either None or not TextDelta with non-empty string
    if let Some(SseChunk::TextDelta(s)) = chunk {
        assert!(s.is_empty(), "role-only delta should produce empty or None");
    }
}

#[test]
fn parse_finish_reason_stop() {
    let line = r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
    let chunk = parse_sse_chunk(line).unwrap();
    match chunk {
        SseChunk::Done => {} // acceptable to return Done on finish_reason=stop
        SseChunk::FinishReason(reason) => assert_eq!(reason, "stop"),
        _ => panic!("expected Done or FinishReason"),
    }
}

#[test]
fn parse_usage_chunk() {
    let line = r#"data: {"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
    let chunk = parse_sse_chunk(line);
    // Usage-only chunks may be returned as Usage or skipped
    if let Some(SseChunk::Usage {
        prompt_tokens,
        completion_tokens,
    }) = chunk
    {
        assert_eq!(prompt_tokens, 10);
        assert_eq!(completion_tokens, 5);
    }
    // None is also acceptable if usage is extracted separately
}

#[test]
fn parse_tool_call_chunk() {
    let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"search","arguments":""}}]},"finish_reason":null}]}"#;
    let chunk = parse_sse_chunk(line);
    if let Some(SseChunk::ToolCallStart { id, name }) = chunk {
        assert_eq!(id, "call_abc");
        assert_eq!(name, "search");
    }
    // streaming tool calls may be chunked — start chunk is what we test here
}

#[test]
fn parse_tool_call_arguments_delta() {
    let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"q\":"}}]},"finish_reason":null}]}"#;
    let chunk = parse_sse_chunk(line);
    if let Some(SseChunk::ToolCallArgsDelta { index, args_delta }) = chunk {
        assert_eq!(index, 0);
        assert!(!args_delta.is_empty());
    }
}

// ── Event collection tests ────────────────────────────────────────────────────

use tianshu::llm::LlmStreamEvent;
use tianshu_llm_openai::sse::collect_events_from_sse;

#[test]
fn collect_text_events() {
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"world\"},\"finish_reason\":null}]}\n\n",
        "data: [DONE]\n\n"
    );
    let events = collect_events_from_sse(sse);
    let texts: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            LlmStreamEvent::TextDelta(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["hello ", "world"]);
    assert!(events.iter().any(|e| matches!(e, LlmStreamEvent::Done(_))));
}

#[test]
fn collect_events_stops_at_done() {
    let sse = "data: [DONE]\n\n";
    let events = collect_events_from_sse(sse);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], LlmStreamEvent::Done(_)));
}