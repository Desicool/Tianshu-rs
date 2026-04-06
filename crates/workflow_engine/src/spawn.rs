// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Configuration for spawning a child workflow.
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub workflow_code: String,
    pub resource_data: Option<JsonValue>,
    /// If None, a key is auto-generated as "{parent_key}_{timestamp}_{workflow_code}".
    pub case_key: Option<String>,
}

/// A handle to a spawned child workflow case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildHandle {
    pub case_key: String,
    pub workflow_code: String,
}

/// Status of a child workflow.
#[derive(Debug, Clone)]
pub enum ChildStatus {
    Running,
    Waiting,
    Finished {
        finished_type: String,
        finished_description: String,
        resource_data: Option<JsonValue>,
    },
    Failed {
        error: String,
    },
}

/// Aggregate result of checking all children.
pub enum ChildrenResult {
    /// All children have reached a terminal state.
    AllDone(Vec<(ChildHandle, ChildStatus)>),
    /// Some children are still in-flight; value is the count of pending children.
    Pending(usize),
}
