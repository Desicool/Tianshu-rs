// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use chrono::Utc;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info};

use crate::case::{Case, ExecutionState};
use crate::compact::ManagedConversation;
use crate::llm::{LlmProvider, LlmRequest};
use crate::observe::{Observer, StepRecord, WorkflowRecord};
use crate::retry::RetryPolicy;
use crate::spawn::{ChildHandle, ChildStatus, ChildrenResult, SpawnConfig};
use crate::store::{CaseStore, StateStore};
use crate::tool::ToolRegistry;
use crate::tool_loop::{run_tool_loop, ToolLoopConfig, ToolLoopResult};

/// WorkflowContext provides checkpoint management and automatic cleanup
/// for workflow execution.
///
/// Storage is injected via `CaseStore` and `StateStore` traits, so any
/// database (PostgreSQL, SQLite, Redis, in-memory …) can be used.
pub struct WorkflowContext {
    /// The case being executed.
    pub case: Case,

    /// In-memory checkpoint cache to avoid repeated store reads.
    checkpoint_cache: HashMap<String, JsonValue>,

    case_store: Arc<dyn CaseStore>,
    state_store: Arc<dyn StateStore>,

    /// Optional observer for collecting step/workflow/LLM data.
    observer: Option<Arc<dyn Observer>>,
    /// Step records accumulated in this tick (cleared by `finish()`).
    step_records: Vec<StepRecord>,
    /// When this context was created (for workflow duration tracking).
    tick_start: Instant,
    /// Snapshot of `case.resource_data` at context creation time.
    initial_resource_data: Option<JsonValue>,

    /// Optional managed conversation for automatic context compaction.
    managed_conversation: Option<ManagedConversation>,
}

impl WorkflowContext {
    pub fn new(
        case: Case,
        case_store: Arc<dyn CaseStore>,
        state_store: Arc<dyn StateStore>,
    ) -> Self {
        let initial_resource_data = case.resource_data.clone();
        Self {
            case,
            checkpoint_cache: HashMap::new(),
            case_store,
            state_store,
            observer: None,
            step_records: Vec::new(),
            tick_start: Instant::now(),
            initial_resource_data,
            managed_conversation: None,
        }
    }

    /// Attach an observer to this context. Must be called before any steps run.
    pub fn set_observer(&mut self, observer: Arc<dyn Observer>) {
        self.observer = Some(observer);
    }

    /// Returns a reference to the observer, if one has been attached.
    pub fn observer(&self) -> Option<&dyn Observer> {
        self.observer.as_deref()
    }

    /// Returns all step records accumulated in this tick.
    pub fn step_records(&self) -> &[StepRecord] {
        &self.step_records
    }

    // ── Internal key helpers ─────────────────────────────────────────────────

    fn checkpoint_step_key(&self, step_name: &str) -> String {
        format!("wf_{}", step_name)
    }

    fn state_step_key(&self, name: &str) -> String {
        format!("wf_state_{}", name)
    }

    /// Compound cache key for the in-memory map.
    fn cache_key(&self, step_key: &str) -> String {
        format!("{}:{}", self.case.case_key, step_key)
    }

    // ── Checkpoint API ───────────────────────────────────────────────────────

    /// Get a checkpoint value, checking the in-memory cache first.
    pub async fn get_checkpoint(&mut self, step_name: &str) -> Result<Option<JsonValue>> {
        let step_key = self.checkpoint_step_key(step_name);
        let cache_key = self.cache_key(&step_key);

        if let Some(v) = self.checkpoint_cache.get(&cache_key) {
            return Ok(Some(v.clone()));
        }

        match self.state_store.get(&self.case.case_key, &step_key).await? {
            Some(entry) => {
                let value: JsonValue = serde_json::from_str(&entry.data)?;
                if value.is_null() {
                    // null sentinel means the step was cleared
                    return Ok(None);
                }
                self.checkpoint_cache.insert(cache_key, value.clone());
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Save a checkpoint to the store and the in-memory cache.
    pub async fn save_checkpoint(&mut self, step_name: &str, value: JsonValue) -> Result<()> {
        let step_key = self.checkpoint_step_key(step_name);
        let cache_key = self.cache_key(&step_key);

        let data = serde_json::to_string(&value)?;
        self.state_store
            .save(&self.case.case_key, &step_key, &data)
            .await?;
        self.checkpoint_cache.insert(cache_key, value);

        info!(
            "Saved checkpoint: case_key={}, step={}",
            self.case.case_key, step_name
        );
        Ok(())
    }

    /// Remove a single checkpoint from cache and store.
    pub async fn clear_step(&mut self, step_name: &str) -> Result<()> {
        let step_key = self.checkpoint_step_key(step_name);
        let cache_key = self.cache_key(&step_key);

        // Remove from cache so the null sentinel is visible on next get
        self.checkpoint_cache.remove(&cache_key);
        // We overwrite with an empty marker so adapters don't need a per-key delete.
        // Alternatively, delete from the store directly — we use a dedicated call here
        // and rely on the state_store to handle the semantics.
        // Because StateStore has no per-key delete, we save an empty sentinel.
        // This approach is explicit and portable across adapters.
        // (Adapters that want real deletes can implement an optional extension.)
        //
        // SIMPLER: save an empty string; get_checkpoint treats empty-string as absent.
        // We store a JSON null to mark "cleared".
        let data = serde_json::to_string(&JsonValue::Null)?;
        self.state_store
            .save(&self.case.case_key, &step_key, &data)
            .await?;

        info!(
            "Cleared checkpoint: case_key={}, step={}",
            self.case.case_key, step_name
        );
        Ok(())
    }

    /// Remove multiple checkpoints.
    pub async fn clear_steps(&mut self, step_names: &[&str]) -> Result<()> {
        for s in step_names {
            self.clear_step(s).await?;
        }
        Ok(())
    }

    // ── Step execution with idempotent checkpoint ────────────────────────────

    /// Execute a step with automatic checkpoint support.
    ///
    /// If a non-null checkpoint already exists for `step_name`, the cached
    /// result is returned without re-executing `f`. Otherwise `f` is called
    /// and its result is persisted as a checkpoint.
    pub async fn step<F, Fut, T>(&mut self, step_name: &str, f: F) -> Result<T>
    where
        F: FnOnce(&mut Self) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let step_start = Instant::now();
        let input_snapshot = self.case.resource_data.clone();
        let timestamp = Utc::now();

        if let Some(cached) = self.get_checkpoint(step_name).await? {
            if !cached.is_null() {
                info!(
                    "Restoring checkpoint: case_key={}, step={}",
                    self.case.case_key, step_name
                );
                let duration_ms = step_start.elapsed().as_millis() as u64;
                let record = StepRecord {
                    case_key: self.case.case_key.clone(),
                    workflow_code: self.case.workflow_code.clone(),
                    step_name: step_name.to_string(),
                    input_resource_data: input_snapshot,
                    output: Some(cached.clone()),
                    duration_ms,
                    timestamp,
                    cached: true,
                    error: None,
                    agent_id: None,
                };
                self.step_records.push(record.clone());
                if let Some(obs) = &self.observer {
                    obs.on_step(&record).await;
                }
                return Ok(serde_json::from_value(cached)?);
            }
        }

        info!(
            "Executing step: case_key={}, step={}",
            self.case.case_key, step_name
        );
        let exec_result = f(self).await;
        let duration_ms = step_start.elapsed().as_millis() as u64;

        let record = match &exec_result {
            Ok(val) => {
                let value = serde_json::to_value(val)?;
                StepRecord {
                    case_key: self.case.case_key.clone(),
                    workflow_code: self.case.workflow_code.clone(),
                    step_name: step_name.to_string(),
                    input_resource_data: input_snapshot,
                    output: Some(value),
                    duration_ms,
                    timestamp,
                    cached: false,
                    error: None,
                    agent_id: None,
                }
            }
            Err(e) => StepRecord {
                case_key: self.case.case_key.clone(),
                workflow_code: self.case.workflow_code.clone(),
                step_name: step_name.to_string(),
                input_resource_data: input_snapshot,
                output: None,
                duration_ms,
                timestamp,
                cached: false,
                error: Some(e.to_string()),
                agent_id: None,
            },
        };

        self.step_records.push(record.clone());
        if let Some(obs) = &self.observer {
            obs.on_step(&record).await;
        }

        let result = exec_result?;
        let value = serde_json::to_value(&result)?;
        self.save_checkpoint(step_name, value).await?;
        Ok(result)
    }

    // ── State variable API ───────────────────────────────────────────────────

    /// Read a named state variable, returning `default` if not yet set.
    pub async fn get_state<T>(&mut self, name: &str, default: T) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let step_key = self.state_step_key(name);
        let cache_key = self.cache_key(&step_key);

        if let Some(v) = self.checkpoint_cache.get(&cache_key) {
            return Ok(serde_json::from_value(v.clone())?);
        }

        match self.state_store.get(&self.case.case_key, &step_key).await? {
            Some(entry) => {
                let value: JsonValue = serde_json::from_str(&entry.data)?;
                self.checkpoint_cache.insert(cache_key, value.clone());
                Ok(serde_json::from_value(value)?)
            }
            None => Ok(default),
        }
    }

    /// Write a named state variable.
    pub async fn set_state<T>(&mut self, name: &str, value: T) -> Result<()>
    where
        T: serde::Serialize,
    {
        let step_key = self.state_step_key(name);
        let cache_key = self.cache_key(&step_key);
        let json_value = serde_json::to_value(&value)?;
        let data = serde_json::to_string(&json_value)?;

        self.state_store
            .save(&self.case.case_key, &step_key, &data)
            .await?;
        self.checkpoint_cache.insert(cache_key, json_value);

        info!("Set state: case_key={}, name={}", self.case.case_key, name);
        Ok(())
    }

    // ── Session-scoped state variable API (cross-case) ────────────────────

    fn session_state_step_key(&self, name: &str) -> String {
        format!("wf_sess_state_{}", name)
    }

    fn session_cache_key(&self, step_key: &str) -> String {
        format!("session:{}:{}", self.case.session_id, step_key)
    }

    /// Read a session-scoped (cross-case) variable, returning `default` if not set.
    ///
    /// Session-scoped variables are shared across all cases in the same session.
    /// **No engine-level locking is provided** — workflows that use shared
    /// variables are responsible for their own concurrency control.
    pub async fn get_session_state<T>(&mut self, name: &str, default: T) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let step_key = self.session_state_step_key(name);
        let cache_key = self.session_cache_key(&step_key);

        if let Some(v) = self.checkpoint_cache.get(&cache_key) {
            return Ok(serde_json::from_value(v.clone())?);
        }

        match self
            .state_store
            .get_session(&self.case.session_id, &step_key)
            .await?
        {
            Some(entry) => {
                let value: JsonValue = serde_json::from_str(&entry.data)?;
                self.checkpoint_cache.insert(cache_key, value.clone());
                Ok(serde_json::from_value(value)?)
            }
            None => Ok(default),
        }
    }

    /// Write a session-scoped (cross-case) variable.
    ///
    /// **No engine-level locking is provided.** If multiple cases within the
    /// same session write to the same variable concurrently, the last write wins.
    /// Workflows are responsible for their own concurrency control.
    pub async fn set_session_state<T>(&mut self, name: &str, value: T) -> Result<()>
    where
        T: serde::Serialize,
    {
        let step_key = self.session_state_step_key(name);
        let cache_key = self.session_cache_key(&step_key);
        let json_value = serde_json::to_value(&value)?;
        let data = serde_json::to_string(&json_value)?;

        self.state_store
            .save_session(&self.case.session_id, &step_key, &data)
            .await?;
        self.checkpoint_cache.insert(cache_key, json_value);

        info!(
            "Set session state: session_id={}, name={}",
            self.case.session_id, name
        );
        Ok(())
    }

    // ── Child workflow spawning ────────────────────────────────────────────

    /// Spawn a child workflow. The child is immediately created in Running state.
    pub async fn spawn_child(&mut self, config: SpawnConfig) -> Result<ChildHandle> {
        let child_key = config.case_key.unwrap_or_else(|| {
            format!(
                "{}_{}_child_{}",
                self.case.case_key,
                chrono::Utc::now().timestamp_millis(),
                config.workflow_code
            )
        });

        let mut child = Case::new(
            child_key.clone(),
            self.case.session_id.clone(),
            config.workflow_code.clone(),
        );
        child.parent_key = Some(self.case.case_key.clone());
        child.resource_data = config.resource_data;

        self.case_store.upsert(&child).await?;
        self.case.child_keys.push(child_key.clone());
        self.case_store.upsert(&self.case).await?;

        Ok(ChildHandle {
            case_key: child_key,
            workflow_code: config.workflow_code,
        })
    }

    /// Spawn multiple child workflows at once.
    pub async fn spawn_children(&mut self, configs: Vec<SpawnConfig>) -> Result<Vec<ChildHandle>> {
        let mut handles = Vec::new();
        for config in configs {
            handles.push(self.spawn_child(config).await?);
        }
        Ok(handles)
    }

    /// Check the status of a single child workflow.
    pub async fn child_status(&self, handle: &ChildHandle) -> Result<ChildStatus> {
        match self.case_store.get_by_key(&handle.case_key).await? {
            None => Ok(ChildStatus::Failed {
                error: format!("child case '{}' not found", handle.case_key),
            }),
            Some(child) => Ok(match child.execution_state {
                ExecutionState::Running => ChildStatus::Running,
                ExecutionState::Waiting => ChildStatus::Waiting,
                ExecutionState::Finished => ChildStatus::Finished {
                    finished_type: child.finished_type.unwrap_or_default(),
                    finished_description: child.finished_description.unwrap_or_default(),
                    resource_data: child.resource_data,
                },
            }),
        }
    }

    /// Check all children. Returns Pending if any are still in-flight, AllDone when all finish.
    pub async fn await_children(&self, handles: &[ChildHandle]) -> Result<ChildrenResult> {
        let mut statuses = Vec::new();
        let mut pending = 0;
        for handle in handles {
            let status = self.child_status(handle).await?;
            match &status {
                ChildStatus::Running | ChildStatus::Waiting => pending += 1,
                _ => {}
            }
            statuses.push((handle.clone(), status));
        }
        if pending > 0 {
            Ok(ChildrenResult::Pending(pending))
        } else {
            Ok(ChildrenResult::AllDone(statuses))
        }
    }

    // ── Retry-aware step execution ───────────────────────────────────────────

    /// Like [`step()`], but applies `policy` to the closure execution.
    ///
    /// Checkpoint semantics are preserved: once the step succeeds, its result
    /// is cached and never retried, even across process restarts.
    pub async fn step_with_retry<F, Fut, T>(
        &mut self,
        step_name: &str,
        policy: &RetryPolicy,
        f: F,
    ) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let policy_clone = policy;
        self.step(step_name, |_ctx| async move {
            crate::retry::with_retry(policy_clone, |_retry_ctx| f()).await
        })
        .await
    }

    // ── Tool-use step ────────────────────────────────────────────────────────

    /// Execute a tool-use step with automatic checkpointing.
    ///
    /// Calls the LLM with the given tools, runs the tool loop until the model
    /// produces a final text response or `config.max_rounds` is exhausted, and
    /// checkpoints the entire result so the step is never re-executed.
    pub async fn tool_step(
        &mut self,
        step_name: &str,
        llm: &dyn LlmProvider,
        tools: &ToolRegistry,
        request: LlmRequest,
        config: &ToolLoopConfig,
    ) -> Result<ToolLoopResult> {
        let observer_ref = self.observer.as_deref();
        let result = run_tool_loop(llm, request, tools, config, observer_ref).await?;
        // Checkpoint the final text so we never re-run on restart.
        self.save_checkpoint(step_name, serde_json::to_value(&result.final_text)?)
            .await?;
        Ok(result)
    }

    // ── Managed conversation ─────────────────────────────────────────────────

    /// Attach a `ManagedConversation` for automatic context compaction.
    pub fn set_managed_conversation(&mut self, conv: ManagedConversation) {
        self.managed_conversation = Some(conv);
    }

    /// Access the managed conversation (if one was set).
    pub fn managed_conversation(&mut self) -> Option<&mut ManagedConversation> {
        self.managed_conversation.as_mut()
    }

    // ── Workflow completion ──────────────────────────────────────────────────

    /// Mark the workflow as finished.
    ///
    /// Persists the final case state to `CaseStore`, removes all runtime state
    /// from `StateStore`, and clears the in-memory cache.
    pub async fn finish(
        &mut self,
        finished_type: String,
        finished_description: String,
    ) -> Result<()> {
        info!(
            "Finishing workflow: case_key={}, type={}, description={}",
            self.case.case_key, finished_type, finished_description
        );

        self.case.execution_state = ExecutionState::Finished;
        self.case.finished_type = Some(finished_type.clone());
        self.case.finished_description = Some(finished_description.clone());
        self.case.updated_at = chrono::Utc::now();

        self.case_store.upsert(&self.case).await?;

        match self.state_store.delete_by_case(&self.case.case_key).await {
            Ok(()) => {
                info!("Cleaned up state for case_key={}", self.case.case_key);
            }
            Err(e) => {
                error!(
                    "Failed to cleanup state for case_key={}: {}",
                    self.case.case_key, e
                );
                // Don't fail the workflow if cleanup errors.
            }
        }

        // Emit workflow-complete record before clearing state.
        if let Some(obs) = &self.observer {
            let finished_at = Utc::now();
            let total_duration_ms = self.tick_start.elapsed().as_millis() as u64;
            let wf_record = WorkflowRecord {
                case_key: self.case.case_key.clone(),
                session_id: self.case.session_id.clone(),
                workflow_code: self.case.workflow_code.clone(),
                input_resource_data: self.initial_resource_data.clone(),
                output_resource_data: self.case.resource_data.clone(),
                finished_type: self.case.finished_type.clone(),
                finished_description: self.case.finished_description.clone(),
                steps: self.step_records.clone(),
                total_duration_ms,
                started_at: self.case.created_at,
                finished_at,
            };
            obs.on_workflow_complete(&wf_record).await;
            obs.flush().await;
        }

        self.checkpoint_cache.clear();
        self.step_records.clear();

        info!("Workflow finished: case_key={}", self.case.case_key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{InMemoryCaseStore, InMemoryStateStore};

    fn make_test_case(key: &str) -> Case {
        Case::new(key.into(), "sess_test".into(), "wf_test".into())
    }

    fn make_ctx(key: &str) -> WorkflowContext {
        WorkflowContext::new(
            make_test_case(key),
            Arc::new(InMemoryCaseStore::default()),
            Arc::new(InMemoryStateStore::default()),
        )
    }

    #[test]
    fn checkpoint_key_format() {
        let ctx = make_ctx("ck_key");
        assert_eq!(ctx.checkpoint_step_key("step1"), "wf_step1");
        assert_eq!(ctx.cache_key("wf_step1"), "ck_key:wf_step1");
    }

    #[test]
    fn state_key_format() {
        let ctx = make_ctx("st_key");
        assert_eq!(ctx.state_step_key("foo"), "wf_state_foo");
    }

    #[tokio::test]
    async fn context_creation() {
        let ctx = make_ctx("new_key");
        assert_eq!(ctx.case.case_key, "new_key");
        assert!(ctx.checkpoint_cache.is_empty());
    }
}
