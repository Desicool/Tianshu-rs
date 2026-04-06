// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use std::sync::RwLock;
use tianshu::observe::{LlmCallRecord, Observer, StepRecord, WorkflowRecord};

/// In-memory observer that accumulates all records for programmatic access.
///
/// Useful for testing, post-run analysis, and building RLHF datasets.
/// All accessor methods return cloned snapshots.
///
/// # Example
///
/// ```rust,ignore
/// let obs = Arc::new(InMemoryObserver::new());
/// scheduler.set_observer(obs.clone());
/// // ... run workflows ...
/// let entries = step_dataset(&obs.step_records_for_case("case_123"));
/// ```
#[derive(Default)]
pub struct InMemoryObserver {
    steps: RwLock<Vec<StepRecord>>,
    workflows: RwLock<Vec<WorkflowRecord>>,
    llm_calls: RwLock<Vec<LlmCallRecord>>,
}

impl InMemoryObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns all collected step records.
    pub fn step_records(&self) -> Vec<StepRecord> {
        self.steps.read().unwrap().clone()
    }

    /// Returns all collected workflow-complete records.
    pub fn workflow_records(&self) -> Vec<WorkflowRecord> {
        self.workflows.read().unwrap().clone()
    }

    /// Returns all collected LLM call records.
    pub fn llm_records(&self) -> Vec<LlmCallRecord> {
        self.llm_calls.read().unwrap().clone()
    }

    /// Returns step records for a specific case (across all ticks).
    pub fn step_records_for_case(&self, case_key: &str) -> Vec<StepRecord> {
        self.steps
            .read()
            .unwrap()
            .iter()
            .filter(|r| r.case_key == case_key)
            .cloned()
            .collect()
    }

    /// Clear all accumulated records.
    pub fn clear(&self) {
        self.steps.write().unwrap().clear();
        self.workflows.write().unwrap().clear();
        self.llm_calls.write().unwrap().clear();
    }
}

#[async_trait]
impl Observer for InMemoryObserver {
    async fn on_step(&self, record: &StepRecord) {
        self.steps.write().unwrap().push(record.clone());
    }

    async fn on_workflow_complete(&self, record: &WorkflowRecord) {
        self.workflows.write().unwrap().push(record.clone());
    }

    async fn on_llm_call(&self, record: &LlmCallRecord) {
        self.llm_calls.write().unwrap().push(record.clone());
    }
}
