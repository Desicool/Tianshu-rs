// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tianshu::{
    observe::{LlmCallRecord, StepRecord, WorkflowRecord},
    LlmMessage, LlmRequest, LlmUsage, Observer,
};
use tianshu_observe::JsonlObserver;

fn step() -> StepRecord {
    StepRecord {
        case_key: "case1".to_string(),
        workflow_code: "wf".to_string(),
        step_name: "compute".to_string(),
        input_resource_data: Some(json!({"x": 1})),
        output: Some(json!(42)),
        duration_ms: 10,
        timestamp: Utc::now(),
        cached: false,
        error: None,
    }
}

fn workflow() -> WorkflowRecord {
    WorkflowRecord {
        case_key: "case1".to_string(),
        session_id: "sess".to_string(),
        workflow_code: "wf".to_string(),
        input_resource_data: Some(json!({"input": "x"})),
        output_resource_data: Some(json!({"output": "y"})),
        finished_type: Some("success".to_string()),
        finished_description: None,
        steps: vec![],
        total_duration_ms: 100,
        started_at: Utc::now(),
        finished_at: Utc::now(),
    }
}

fn llm_call() -> LlmCallRecord {
    LlmCallRecord {
        case_key: "case1".to_string(),
        step_name: Some("classify".to_string()),
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
            prompt_tokens: 10,
            completion_tokens: 5,
        }),
        duration_ms: 200,
        timestamp: Utc::now(),
        error: None,
    }
}

fn read_jsonl(path: &str) -> Vec<Value> {
    let content = std::fs::read_to_string(path).unwrap();
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn writes_step_as_jsonl_line() {
    let file = NamedTempFile::new().unwrap();
    let obs = JsonlObserver::new(file.path().to_str().unwrap())
        .await
        .unwrap();

    obs.on_step(&step()).await;
    obs.flush().await;

    let lines = read_jsonl(file.path().to_str().unwrap());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["type"], "step");
    assert_eq!(lines[0]["step_name"], "compute");
    assert_eq!(lines[0]["case_key"], "case1");
    assert!(!lines[0]["cached"].as_bool().unwrap());
}

#[tokio::test]
async fn writes_workflow_complete_as_jsonl_line() {
    let file = NamedTempFile::new().unwrap();
    let obs = JsonlObserver::new(file.path().to_str().unwrap())
        .await
        .unwrap();

    obs.on_workflow_complete(&workflow()).await;
    obs.flush().await;

    let lines = read_jsonl(file.path().to_str().unwrap());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["type"], "workflow_complete");
    assert_eq!(lines[0]["workflow_code"], "wf");
}

#[tokio::test]
async fn writes_llm_call_as_jsonl_line() {
    let file = NamedTempFile::new().unwrap();
    let obs = JsonlObserver::new(file.path().to_str().unwrap())
        .await
        .unwrap();

    obs.on_llm_call(&llm_call()).await;
    obs.flush().await;

    let lines = read_jsonl(file.path().to_str().unwrap());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["type"], "llm_call");
    assert_eq!(lines[0]["model"], "gpt-4");
}

#[tokio::test]
async fn multiple_events_each_on_own_line() {
    let file = NamedTempFile::new().unwrap();
    let obs = JsonlObserver::new(file.path().to_str().unwrap())
        .await
        .unwrap();

    obs.on_step(&step()).await;
    obs.on_workflow_complete(&workflow()).await;
    obs.on_llm_call(&llm_call()).await;
    obs.flush().await;

    let lines = read_jsonl(file.path().to_str().unwrap());
    assert_eq!(lines.len(), 3);
    let types: Vec<&str> = lines.iter().map(|l| l["type"].as_str().unwrap()).collect();
    assert!(types.contains(&"step"));
    assert!(types.contains(&"workflow_complete"));
    assert!(types.contains(&"llm_call"));
}

#[tokio::test]
async fn creates_file_if_not_exists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new_output.jsonl");
    assert!(!path.exists());

    let obs = JsonlObserver::new(path.to_str().unwrap()).await.unwrap();
    obs.on_step(&step()).await;
    obs.flush().await;

    assert!(path.exists());
}

#[tokio::test]
async fn usable_as_arc_dyn_observer() {
    let file = NamedTempFile::new().unwrap();
    let obs: Arc<dyn Observer> = Arc::new(
        JsonlObserver::new(file.path().to_str().unwrap())
            .await
            .unwrap(),
    );
    obs.on_step(&step()).await;
    obs.flush().await;
}