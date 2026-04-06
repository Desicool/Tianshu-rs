// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use workflow_engine::Observer;
use workflow_engine::{
    observe::{LlmCallRecord, StepRecord, WorkflowRecord},
    LlmMessage, LlmRequest, LlmUsage,
};
use workflow_engine_observe::InMemoryObserver;

fn make_step(case_key: &str, step_name: &str, cached: bool) -> StepRecord {
    StepRecord {
        case_key: case_key.to_string(),
        workflow_code: "wf".to_string(),
        step_name: step_name.to_string(),
        input_resource_data: None,
        output: Some(json!("ok")),
        duration_ms: 5,
        timestamp: Utc::now(),
        cached,
        error: None,
    }
}

fn make_workflow(case_key: &str) -> WorkflowRecord {
    WorkflowRecord {
        case_key: case_key.to_string(),
        session_id: "sess".to_string(),
        workflow_code: "wf".to_string(),
        input_resource_data: None,
        output_resource_data: Some(json!({"result": "done"})),
        finished_type: Some("success".to_string()),
        finished_description: Some("done".to_string()),
        steps: vec![],
        total_duration_ms: 100,
        started_at: Utc::now(),
        finished_at: Utc::now(),
    }
}

fn make_llm(case_key: &str) -> LlmCallRecord {
    LlmCallRecord {
        case_key: case_key.to_string(),
        step_name: Some("step1".to_string()),
        model: "gpt-4".to_string(),
        request: LlmRequest {
            model: "gpt-4".to_string(),
            system_prompt: None,
            messages: vec![LlmMessage {
                role: "user".to_string(),
                content: "hello".to_string(),

                tool_calls: None,

                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,

            tools: None,
        },
        response_content: Some("hi".to_string()),
        usage: Some(LlmUsage {
            prompt_tokens: 5,
            completion_tokens: 3,
        }),
        duration_ms: 200,
        timestamp: Utc::now(),
        error: None,
    }
}

#[tokio::test]
async fn collects_step_records() {
    let obs = InMemoryObserver::new();
    obs.on_step(&make_step("case1", "s1", false)).await;
    obs.on_step(&make_step("case1", "s2", false)).await;

    let steps = obs.step_records();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].step_name, "s1");
    assert_eq!(steps[1].step_name, "s2");
}

#[tokio::test]
async fn collects_workflow_records() {
    let obs = InMemoryObserver::new();
    obs.on_workflow_complete(&make_workflow("case2")).await;

    let wfs = obs.workflow_records();
    assert_eq!(wfs.len(), 1);
    assert_eq!(wfs[0].case_key, "case2");
    assert_eq!(wfs[0].finished_type.as_deref(), Some("success"));
}

#[tokio::test]
async fn collects_llm_records() {
    let obs = InMemoryObserver::new();
    obs.on_llm_call(&make_llm("case3")).await;

    let calls = obs.llm_records();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].case_key, "case3");
}

#[tokio::test]
async fn step_records_for_case_filters_by_key() {
    let obs = InMemoryObserver::new();
    obs.on_step(&make_step("case_a", "s1", false)).await;
    obs.on_step(&make_step("case_b", "s2", false)).await;
    obs.on_step(&make_step("case_a", "s3", true)).await;

    let for_a = obs.step_records_for_case("case_a");
    assert_eq!(for_a.len(), 2);
    assert!(for_a.iter().all(|r| r.case_key == "case_a"));

    let for_b = obs.step_records_for_case("case_b");
    assert_eq!(for_b.len(), 1);
}

#[tokio::test]
async fn clear_resets_all_records() {
    let obs = InMemoryObserver::new();
    obs.on_step(&make_step("c", "s", false)).await;
    obs.on_workflow_complete(&make_workflow("c")).await;
    obs.on_llm_call(&make_llm("c")).await;

    obs.clear();

    assert!(obs.step_records().is_empty());
    assert!(obs.workflow_records().is_empty());
    assert!(obs.llm_records().is_empty());
}

#[tokio::test]
async fn works_as_arc_dyn_observer() {
    // Verify the type can be used as Arc<dyn Observer>
    let obs: Arc<dyn Observer> = Arc::new(InMemoryObserver::new());
    obs.on_step(&make_step("cx", "sx", false)).await;
    obs.flush().await; // no-op, shouldn't panic
}