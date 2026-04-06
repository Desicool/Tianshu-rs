// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Value as JsonValue};
use tianshu::observe::{LlmCallRecord, StepRecord, WorkflowRecord};

/// A single input/output pair suitable for RLHF training datasets.
#[derive(Debug, Clone)]
pub struct DatasetEntry {
    /// The input to the model/step/workflow.
    pub input: JsonValue,
    /// The output produced.
    pub output: JsonValue,
    /// Supplementary metadata (workflow_code, step_name, case_key, etc.).
    pub metadata: JsonValue,
}

/// Convert workflow-complete records to dataset entries.
///
/// Each entry maps `input_resource_data` → `output_resource_data`.
/// Records where either field is `None` are skipped (no meaningful I/O pair).
pub fn workflow_dataset(records: &[WorkflowRecord]) -> Vec<DatasetEntry> {
    records
        .iter()
        .filter_map(|r| {
            let input = r.input_resource_data.clone()?;
            let output = r.output_resource_data.clone()?;
            Some(DatasetEntry {
                input,
                output,
                metadata: json!({
                    "case_key": r.case_key,
                    "session_id": r.session_id,
                    "workflow_code": r.workflow_code,
                    "finished_type": r.finished_type,
                    "total_duration_ms": r.total_duration_ms,
                }),
            })
        })
        .collect()
}

/// Convert step records to dataset entries.
///
/// Only non-cached (`cached: false`) records with a successful output are included.
/// For RLHF, cached steps are replays of previously-seen executions — use the originals.
pub fn step_dataset(records: &[StepRecord]) -> Vec<DatasetEntry> {
    records
        .iter()
        .filter(|r| !r.cached && r.output.is_some() && r.error.is_none())
        .map(|r| {
            let input = r.input_resource_data.clone().unwrap_or(JsonValue::Null);
            let output = r.output.clone().unwrap(); // safe: filtered above
            DatasetEntry {
                input,
                output,
                metadata: json!({
                    "case_key": r.case_key,
                    "workflow_code": r.workflow_code,
                    "step_name": r.step_name,
                    "duration_ms": r.duration_ms,
                }),
            }
        })
        .collect()
}

/// Convert LLM call records to dataset entries.
///
/// Input is the full request (messages + system prompt).
/// Output is `{"content": "..."}`.
/// Error calls are skipped.
pub fn llm_dataset(records: &[LlmCallRecord]) -> Vec<DatasetEntry> {
    records
        .iter()
        .filter(|r| r.response_content.is_some() && r.error.is_none())
        .map(|r| {
            let messages_json: Vec<JsonValue> = r
                .request
                .messages
                .iter()
                .map(|m| json!({"role": m.role, "content": m.content}))
                .collect();

            let input = json!({
                "model": r.request.model,
                "system_prompt": r.request.system_prompt,
                "messages": messages_json,
                "temperature": r.request.temperature,
                "max_tokens": r.request.max_tokens,
            });

            let output = json!({
                "content": r.response_content,
            });

            DatasetEntry {
                input,
                output,
                metadata: json!({
                    "case_key": r.case_key,
                    "step_name": r.step_name,
                    "model": r.model,
                    "duration_ms": r.duration_ms,
                    "usage": r.usage,
                }),
            }
        })
        .collect()
}