// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use tianshu::{
    observe::{LlmCallRecord, StepRecord, WorkflowRecord},
    LlmMessage, LlmRequest, LlmUsage, Observer,
};
use tianshu_observe::{CompositeObserver, InMemoryObserver};

fn step(case_key: &str) -> StepRecord {
    StepRecord {
        case_key: case_key.to_string(),
        workflow_code: "wf".to_string(),
        step_name: "s".to_string(),
        input_resource_data: None,
        output: Some(json!("x")),
        duration_ms: 1,
        timestamp: Utc::now(),
        cached: false,
        error: None,
    }
}

fn workflow(case_key: &str) -> WorkflowRecord {
    WorkflowRecord {
        case_key: case_key.to_string(),
        session_id: "sess".to_string(),
        workflow_code: "wf".to_string(),
        input_resource_data: None,
        output_resource_data: None,
        finished_type: Some("ok".to_string()),
        finished_description: None,
        steps: vec![],
        total_duration_ms: 10,
        started_at: Utc::now(),
        finished_at: Utc::now(),
    }
}

fn llm_call(case_key: &str) -> LlmCallRecord {
    LlmCallRecord {
        case_key: case_key.to_string(),
        step_name: None,
        model: "m".to_string(),
        request: LlmRequest {
            model: "m".to_string(),
            system_prompt: None,
            messages: vec![LlmMessage {
                role: "user".to_string(),
                content: "hi".to_string(),

                tool_calls: None,

                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,

            tools: None,
        },
        response_content: Some("ok".to_string()),
        usage: Some(LlmUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
        }),
        duration_ms: 50,
        timestamp: Utc::now(),
        error: None,
    }
}

#[tokio::test]
async fn fans_out_step_to_all_children() {
    let a = Arc::new(InMemoryObserver::new());
    let b = Arc::new(InMemoryObserver::new());
    let composite = CompositeObserver::new(vec![a.clone(), b.clone()]);

    composite.on_step(&step("case1")).await;

    assert_eq!(a.step_records().len(), 1);
    assert_eq!(b.step_records().len(), 1);
}

#[tokio::test]
async fn fans_out_workflow_complete() {
    let a = Arc::new(InMemoryObserver::new());
    let b = Arc::new(InMemoryObserver::new());
    let composite = CompositeObserver::new(vec![a.clone(), b.clone()]);

    composite.on_workflow_complete(&workflow("case2")).await;

    assert_eq!(a.workflow_records().len(), 1);
    assert_eq!(b.workflow_records().len(), 1);
}

#[tokio::test]
async fn fans_out_llm_call() {
    let a = Arc::new(InMemoryObserver::new());
    let b = Arc::new(InMemoryObserver::new());
    let composite = CompositeObserver::new(vec![a.clone(), b.clone()]);

    composite.on_llm_call(&llm_call("case3")).await;

    assert_eq!(a.llm_records().len(), 1);
    assert_eq!(b.llm_records().len(), 1);
}

#[tokio::test]
async fn flush_calls_all_children() {
    // Both children implement flush as no-op; just ensure it doesn't panic
    let a = Arc::new(InMemoryObserver::new());
    let b = Arc::new(InMemoryObserver::new());
    let composite = CompositeObserver::new(vec![a.clone(), b.clone()]);
    composite.flush().await; // should not panic
}

#[tokio::test]
async fn empty_composite_is_valid() {
    let composite = CompositeObserver::new(vec![]);
    composite.on_step(&step("cx")).await; // should not panic
}

#[tokio::test]
async fn usable_as_arc_dyn_observer() {
    let obs: Arc<dyn Observer> = Arc::new(CompositeObserver::new(vec![Arc::new(
        InMemoryObserver::new(),
    )]));
    obs.on_step(&step("c")).await;
}