// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use std::sync::Arc;
use tianshu::observe::{LlmCallRecord, Observer, StepRecord, WorkflowRecord};

/// An observer that fans out every event to a list of child observers.
///
/// Useful for combining multiple backends simultaneously, e.g.:
///
/// ```rust,ignore
/// let obs = CompositeObserver::new(vec![
///     Arc::new(InMemoryObserver::new()),
///     Arc::new(JsonlObserver::new("traces.jsonl").await.unwrap()),
/// ]);
/// scheduler.set_observer(Arc::new(obs));
/// ```
pub struct CompositeObserver {
    children: Vec<Arc<dyn Observer>>,
}

impl CompositeObserver {
    pub fn new(children: Vec<Arc<dyn Observer>>) -> Self {
        Self { children }
    }
}

#[async_trait]
impl Observer for CompositeObserver {
    async fn on_step(&self, record: &StepRecord) {
        for child in &self.children {
            child.on_step(record).await;
        }
    }

    async fn on_workflow_complete(&self, record: &WorkflowRecord) {
        for child in &self.children {
            child.on_workflow_complete(record).await;
        }
    }

    async fn on_llm_call(&self, record: &LlmCallRecord) {
        for child in &self.children {
            child.on_llm_call(record).await;
        }
    }

    async fn flush(&self) {
        for child in &self.children {
            child.flush().await;
        }
    }
}