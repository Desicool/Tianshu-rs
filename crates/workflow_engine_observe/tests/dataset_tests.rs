// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use chrono::Utc;
use serde_json::json;
use workflow_engine::{
    observe::{LlmCallRecord, StepRecord, WorkflowRecord},
    LlmMessage, LlmRequest, LlmUsage,
};
use workflow_engine_observe::dataset::{llm_dataset, step_dataset, workflow_dataset};

fn step(step_name: &str, cached: bool, output: serde_json::Value) -> StepRecord {
    StepRecord {
        case_key: "case1".to_string(),
        workflow_code: "approval".to_string(),
        step_name: step_name.to_string(),
        input_resource_data: Some(json!({"doc": "test"})),
        output: Some(output),
        duration_ms: 10,
        timestamp: Utc::now(),
        cached,
        error: None,
    }
}

fn workflow_with_steps(steps: Vec<StepRecord>) -> WorkflowRecord {
    WorkflowRecord {
        case_key: "case1".to_string(),
        session_id: "sess".to_string(),
        workflow_code: "approval".to_string(),
        input_resource_data: Some(json!({"doc_id": "abc"})),
        output_resource_data: Some(json!({"result": "approved"})),
        finished_type: Some("success".to_string()),
        finished_description: Some("approved".to_string()),
        steps,
        total_duration_ms: 200,
        started_at: Utc::now(),
        finished_at: Utc::now(),
    }
}

fn llm(model: &str) -> LlmCallRecord {
    LlmCallRecord {
        case_key: "case1".to_string(),
        step_name: Some("classify".to_string()),
        model: model.to_string(),
        request: LlmRequest {
            model: model.to_string(),
            system_prompt: Some("You are helpful".to_string()),
            messages: vec![LlmMessage {
                role: "user".to_string(),
                content: "classify this".to_string(),

                tool_calls: None,

                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,

            tools: None,
        },
        response_content: Some("category: A".to_string()),
        usage: Some(LlmUsage {
            prompt_tokens: 20,
            completion_tokens: 5,
        }),
        duration_ms: 300,
        timestamp: Utc::now(),
        error: None,
    }
}

// ── workflow_dataset ─────────────────────────────────────────────────────────

#[test]
fn workflow_dataset_entry_has_input_output() {
    let wf = workflow_with_steps(vec![]);
    let entries = workflow_dataset(&[wf]);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].input, json!({"doc_id": "abc"}));
    assert_eq!(entries[0].output, json!({"result": "approved"}));
}

#[test]
fn workflow_dataset_metadata_contains_workflow_code() {
    let wf = workflow_with_steps(vec![]);
    let entries = workflow_dataset(&[wf]);
    assert_eq!(entries[0].metadata["workflow_code"], "approval");
    assert_eq!(entries[0].metadata["case_key"], "case1");
    assert_eq!(entries[0].metadata["finished_type"], "success");
}

#[test]
fn workflow_dataset_skips_null_input_or_output() {
    let mut wf = workflow_with_steps(vec![]);
    wf.input_resource_data = None;
    // No input → skip (can't form a meaningful I/O pair)
    let entries = workflow_dataset(&[wf]);
    assert!(entries.is_empty());
}

// ── step_dataset ─────────────────────────────────────────────────────────────

#[test]
fn step_dataset_only_includes_non_cached() {
    let steps = vec![
        step("s1", false, json!("result1")),
        step("s2", true, json!("result2")), // cached — skip
        step("s3", false, json!("result3")),
    ];
    let entries = step_dataset(&steps);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].metadata["step_name"], "s1");
    assert_eq!(entries[1].metadata["step_name"], "s3");
}

#[test]
fn step_dataset_entry_has_input_output() {
    let steps = vec![step("compute", false, json!(42))];
    let entries = step_dataset(&steps);
    assert_eq!(entries[0].input, json!({"doc": "test"}));
    assert_eq!(entries[0].output, json!(42));
}

#[test]
fn step_dataset_skips_error_steps() {
    let mut bad = step("fail", false, json!(null));
    bad.output = None;
    bad.error = Some("boom".to_string());

    let entries = step_dataset(&[bad]);
    assert!(entries.is_empty());
}

// ── llm_dataset ──────────────────────────────────────────────────────────────

#[test]
fn llm_dataset_has_request_response() {
    let entries = llm_dataset(&[llm("gpt-4")]);
    assert_eq!(entries.len(), 1);

    // Input should contain messages
    assert!(entries[0].input.get("messages").is_some());
    // Output should contain the response text
    assert_eq!(entries[0].output["content"], "category: A");
}

#[test]
fn llm_dataset_metadata_contains_model() {
    let entries = llm_dataset(&[llm("gpt-4")]);
    assert_eq!(entries[0].metadata["model"], "gpt-4");
    assert_eq!(entries[0].metadata["step_name"], "classify");
}

#[test]
fn llm_dataset_skips_error_calls() {
    let mut bad = llm("gpt-4");
    bad.response_content = None;
    bad.error = Some("timeout".to_string());

    let entries = llm_dataset(&[bad]);
    assert!(entries.is_empty());
}