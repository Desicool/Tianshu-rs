use anyhow::Result;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

use crate::case::{Case, ExecutionState};
use crate::store::{CaseStore, StateStore};

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
}

impl WorkflowContext {
    pub fn new(
        case: Case,
        case_store: Arc<dyn CaseStore>,
        state_store: Arc<dyn StateStore>,
    ) -> Self {
        Self {
            case,
            checkpoint_cache: HashMap::new(),
            case_store,
            state_store,
        }
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
        if let Some(cached) = self.get_checkpoint(step_name).await? {
            if !cached.is_null() {
                info!(
                    "Restoring checkpoint: case_key={}, step={}",
                    self.case.case_key, step_name
                );
                return Ok(serde_json::from_value(cached)?);
            }
        }

        info!(
            "Executing step: case_key={}, step={}",
            self.case.case_key, step_name
        );
        let result = f(self).await?;
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

        info!(
            "Set state: case_key={}, name={}",
            self.case.case_key, name
        );
        Ok(())
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

        match self
            .state_store
            .delete_by_case(&self.case.case_key)
            .await
        {
            Ok(()) => {
                info!(
                    "Cleaned up state for case_key={}",
                    self.case.case_key
                );
            }
            Err(e) => {
                error!(
                    "Failed to cleanup state for case_key={}: {}",
                    self.case.case_key, e
                );
                // Don't fail the workflow if cleanup errors.
            }
        }

        self.checkpoint_cache.clear();

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
        assert_eq!(
            ctx.cache_key("wf_step1"),
            "ck_key:wf_step1"
        );
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
