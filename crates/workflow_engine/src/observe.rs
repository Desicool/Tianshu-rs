use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::time::Instant;

use crate::llm::{LlmProvider, LlmRequest, LlmResponse, LlmUsage};

// ── Data types ────────────────────────────────────────────────────────────────

/// Record of a single step execution.
///
/// Emitted by `WorkflowContext::step()` via `Observer::on_step()`.
/// `cached: true` means the step was restored from a prior checkpoint —
/// useful for RLHF filtering (keep only `cached: false` for original executions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub case_key: String,
    pub workflow_code: String,
    pub step_name: String,
    /// Snapshot of `case.resource_data` immediately before the step ran.
    pub input_resource_data: Option<JsonValue>,
    /// Serialized step result. `None` if the step returned an error.
    pub output: Option<JsonValue>,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
    /// `true` if the result was restored from a checkpoint (replay), `false` if freshly executed.
    pub cached: bool,
    pub error: Option<String>,
}

/// Record emitted when a workflow finishes via `ctx.finish()`.
///
/// `steps` contains all steps that ran (or replayed) in the current tick.
/// Because of the coroutine-like replay model, the final tick always
/// re-runs every step — so `steps` is the complete list for the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRecord {
    pub case_key: String,
    pub session_id: String,
    pub workflow_code: String,
    /// `case.resource_data` captured when the WorkflowContext was created.
    pub input_resource_data: Option<JsonValue>,
    /// `case.resource_data` at the moment `finish()` was called.
    pub output_resource_data: Option<JsonValue>,
    pub finished_type: Option<String>,
    pub finished_description: Option<String>,
    /// All steps from this tick (includes replayed cached steps).
    pub steps: Vec<StepRecord>,
    pub total_duration_ms: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

/// Record of a single tool call execution.
///
/// Emitted by `run_tool_loop()` via `Observer::on_tool_call()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub case_key: String,
    pub step_name: Option<String>,
    pub tool_name: String,
    pub call_id: String,
    pub input: JsonValue,
    pub output: Option<String>,
    pub is_error: bool,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
}

/// Record of a retry attempt.
///
/// Emitted via `Observer::on_retry()` when the retry loop retries after an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryRecord {
    pub case_key: String,
    pub step_name: Option<String>,
    pub attempt: u32,
    pub error_class: String,
    pub error_message: String,
    pub timestamp: DateTime<Utc>,
}

/// Record of a single LLM call.
///
/// Emitted via `observe_llm_call()` or `ObservedLlmProvider`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallRecord {
    pub case_key: String,
    pub step_name: Option<String>,
    pub model: String,
    pub request: LlmRequest,
    /// Response text. `None` if the call errored.
    pub response_content: Option<String>,
    pub usage: Option<LlmUsage>,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
    pub error: Option<String>,
}

// ── Observer trait ────────────────────────────────────────────────────────────

/// Pluggable sink for workflow observability events.
///
/// Implement this trait to route events to any backend:
/// in-memory buffers, JSONL files, remote tracing services, etc.
///
/// All methods have default no-op implementations so you only override
/// what you need.
///
/// # Example
///
/// ```rust,ignore
/// struct PrintObserver;
///
/// #[async_trait]
/// impl Observer for PrintObserver {
///     async fn on_step(&self, record: &StepRecord) {
///         println!("step: {} -> {:?}", record.step_name, record.output);
///     }
///     async fn on_workflow_complete(&self, record: &WorkflowRecord) {
///         println!("workflow done: {}", record.case_key);
///     }
/// }
/// ```
#[async_trait]
pub trait Observer: Send + Sync {
    /// Called after each step execution (cached or fresh).
    async fn on_step(&self, record: &StepRecord) {
        let _ = record;
    }

    /// Called when a workflow calls `ctx.finish()`.
    async fn on_workflow_complete(&self, record: &WorkflowRecord) {
        let _ = record;
    }

    /// Called after each LLM call tracked via `observe_llm_call` or `ObservedLlmProvider`.
    async fn on_llm_call(&self, record: &LlmCallRecord) {
        let _ = record;
    }

    /// Called after each tool call execution in a tool-use loop.
    async fn on_tool_call(&self, record: &ToolCallRecord) {
        let _ = record;
    }

    /// Called when a retry occurs during error recovery.
    async fn on_retry(&self, record: &RetryRecord) {
        let _ = record;
    }

    /// Flush any buffered writes. Called by `WorkflowContext::finish()`.
    /// Default is a no-op; buffered implementations (e.g. JSONL) should override.
    async fn flush(&self) {}
}

// ── Free function ─────────────────────────────────────────────────────────────

/// Execute an LLM call and record it via `observer`.
///
/// Drop-in replacement for `provider.complete(request)` when you have
/// an observer and want to capture timing + input/output.
///
/// # Example
///
/// ```rust,ignore
/// let response = observe_llm_call(
///     observer.as_ref(),
///     &provider,
///     request,
///     &ctx.case.case_key,
///     Some("my_step"),
/// ).await?;
/// ```
pub async fn observe_llm_call(
    observer: &dyn Observer,
    provider: &dyn LlmProvider,
    request: LlmRequest,
    case_key: &str,
    step_name: Option<&str>,
) -> Result<LlmResponse> {
    let start = Instant::now();
    let timestamp = Utc::now();
    let result = provider.complete(request.clone()).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    let record = match &result {
        Ok(resp) => LlmCallRecord {
            case_key: case_key.to_string(),
            step_name: step_name.map(str::to_string),
            model: request.model.clone(),
            request,
            response_content: Some(resp.content.clone()),
            usage: Some(resp.usage.clone()),
            duration_ms,
            timestamp,
            error: None,
        },
        Err(e) => LlmCallRecord {
            case_key: case_key.to_string(),
            step_name: step_name.map(str::to_string),
            model: request.model.clone(),
            request,
            response_content: None,
            usage: None,
            duration_ms,
            timestamp,
            error: Some(e.to_string()),
        },
    };

    observer.on_llm_call(&record).await;
    result
}

// ── ObservedLlmProvider ───────────────────────────────────────────────────────

/// A `LlmProvider` wrapper that records every call via an `Observer`.
///
/// Useful for transparent instrumentation — wrap your provider once at
/// construction time and every call is automatically observed.
///
/// ```rust,ignore
/// let provider = ObservedLlmProvider::new(
///     Arc::new(OpenAiProvider::new(key, model)),
///     observer.clone(),
///     ctx.case.case_key.clone(),
/// );
/// // Now use `provider` as a normal LlmProvider inside the workflow.
/// let response = provider.complete(request).await?;
/// ```
pub struct ObservedLlmProvider {
    inner: Arc<dyn LlmProvider>,
    observer: Arc<dyn Observer>,
    case_key: String,
    step_name: Option<String>,
}

impl ObservedLlmProvider {
    pub fn new(
        inner: Arc<dyn LlmProvider>,
        observer: Arc<dyn Observer>,
        case_key: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            observer,
            case_key: case_key.into(),
            step_name: None,
        }
    }

    /// Associate this provider with a specific step name for richer records.
    pub fn with_step(mut self, step_name: impl Into<String>) -> Self {
        self.step_name = Some(step_name.into());
        self
    }
}

#[async_trait]
impl LlmProvider for ObservedLlmProvider {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        observe_llm_call(
            self.observer.as_ref(),
            self.inner.as_ref(),
            request,
            &self.case_key,
            self.step_name.as_deref(),
        )
        .await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    // ── Helpers ──────────────────────────────────────────────────────────────

    struct CollectingObserver {
        steps: Mutex<Vec<StepRecord>>,
        workflows: Mutex<Vec<WorkflowRecord>>,
        llm_calls: Mutex<Vec<LlmCallRecord>>,
    }

    impl CollectingObserver {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                steps: Mutex::new(vec![]),
                workflows: Mutex::new(vec![]),
                llm_calls: Mutex::new(vec![]),
            })
        }
    }

    #[async_trait]
    impl Observer for CollectingObserver {
        async fn on_step(&self, record: &StepRecord) {
            self.steps.lock().unwrap().push(record.clone());
        }
        async fn on_workflow_complete(&self, record: &WorkflowRecord) {
            self.workflows.lock().unwrap().push(record.clone());
        }
        async fn on_llm_call(&self, record: &LlmCallRecord) {
            self.llm_calls.lock().unwrap().push(record.clone());
        }
    }

    struct OkLlmProvider;
    #[async_trait]
    impl LlmProvider for OkLlmProvider {
        async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
            Ok(LlmResponse {
                content: format!("response to {}", request.model),
                usage: LlmUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                },
                finish_reason: "stop".into(),

                tool_calls: None,
            })
        }
    }

    struct ErrLlmProvider;
    #[async_trait]
    impl LlmProvider for ErrLlmProvider {
        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse> {
            Err(anyhow::anyhow!("provider unavailable"))
        }
    }

    fn make_request() -> LlmRequest {
        LlmRequest {
            model: "gpt-4".into(),
            system_prompt: Some("You are helpful".into()),
            messages: vec![crate::llm::LlmMessage {
                role: "user".into(),
                content: "Hello".into(),

                tool_calls: None,

                tool_call_id: None,
            }],
            temperature: Some(0.7),
            max_tokens: Some(100),

            tools: None,
        }
    }

    // ── Serialization roundtrip ──────────────────────────────────────────────

    #[test]
    fn step_record_serde_roundtrip() {
        let record = StepRecord {
            case_key: "case_1".into(),
            workflow_code: "approval".into(),
            step_name: "fetch_data".into(),
            input_resource_data: Some(json!({"doc": "test"})),
            output: Some(json!("result")),
            duration_ms: 42,
            timestamp: Utc::now(),
            cached: false,
            error: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: StepRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.case_key, "case_1");
        assert_eq!(back.step_name, "fetch_data");
        assert!(!back.cached);
    }

    #[test]
    fn workflow_record_serde_roundtrip() {
        let record = WorkflowRecord {
            case_key: "case_2".into(),
            session_id: "sess_1".into(),
            workflow_code: "approval".into(),
            input_resource_data: Some(json!({"doc_id": "abc"})),
            output_resource_data: Some(json!({"result": "approved"})),
            finished_type: Some("success".into()),
            finished_description: Some("approved".into()),
            steps: vec![],
            total_duration_ms: 500,
            started_at: Utc::now(),
            finished_at: Utc::now(),
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: WorkflowRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.case_key, "case_2");
        assert_eq!(back.finished_type.as_deref(), Some("success"));
    }

    #[test]
    fn llm_call_record_serde_roundtrip() {
        let record = LlmCallRecord {
            case_key: "case_3".into(),
            step_name: Some("classify".into()),
            model: "gpt-4".into(),
            request: make_request(),
            response_content: Some("classification result".into()),
            usage: Some(LlmUsage {
                prompt_tokens: 20,
                completion_tokens: 10,
            }),
            duration_ms: 300,
            timestamp: Utc::now(),
            error: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: LlmCallRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.case_key, "case_3");
        assert_eq!(back.model, "gpt-4");
        assert!(back.error.is_none());
    }

    // ── observe_llm_call ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn observe_llm_call_records_success() {
        let obs = CollectingObserver::new();
        let provider = OkLlmProvider;
        let request = make_request();

        let result = observe_llm_call(obs.as_ref(), &provider, request, "case_x", Some("step1"))
            .await
            .unwrap();

        assert_eq!(result.finish_reason, "stop");

        let calls = obs.llm_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.case_key, "case_x");
        assert_eq!(call.step_name.as_deref(), Some("step1"));
        assert!(call.response_content.is_some());
        assert!(call.error.is_none());
        assert!(call.duration_ms < 1000); // sanity
    }

    #[tokio::test]
    async fn observe_llm_call_records_error() {
        let obs = CollectingObserver::new();
        let provider = ErrLlmProvider;

        let result =
            observe_llm_call(obs.as_ref(), &provider, make_request(), "case_y", None).await;
        assert!(result.is_err());

        let calls = obs.llm_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert!(call.response_content.is_none());
        assert!(call.usage.is_none());
        assert!(call.error.as_deref().unwrap().contains("unavailable"));
    }

    #[tokio::test]
    async fn observe_llm_call_no_step_name() {
        let obs = CollectingObserver::new();
        observe_llm_call(obs.as_ref(), &OkLlmProvider, make_request(), "case_z", None)
            .await
            .unwrap();
        let calls = obs.llm_calls.lock().unwrap();
        assert!(calls[0].step_name.is_none());
    }

    // ── ObservedLlmProvider ──────────────────────────────────────────────────

    #[tokio::test]
    async fn observed_llm_provider_records_call() {
        let obs = CollectingObserver::new();
        let provider = ObservedLlmProvider::new(Arc::new(OkLlmProvider), obs.clone(), "case_p");

        provider.complete(make_request()).await.unwrap();

        let calls = obs.llm_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].case_key, "case_p");
        assert!(calls[0].step_name.is_none());
    }

    #[tokio::test]
    async fn observed_llm_provider_with_step() {
        let obs = CollectingObserver::new();
        let provider = ObservedLlmProvider::new(Arc::new(OkLlmProvider), obs.clone(), "case_q")
            .with_step("summarize");

        provider.complete(make_request()).await.unwrap();

        let calls = obs.llm_calls.lock().unwrap();
        assert_eq!(calls[0].step_name.as_deref(), Some("summarize"));
    }

    #[tokio::test]
    async fn observed_llm_provider_error_still_records() {
        let obs = CollectingObserver::new();
        let provider = ObservedLlmProvider::new(Arc::new(ErrLlmProvider), obs.clone(), "case_err");

        let result = provider.complete(make_request()).await;
        assert!(result.is_err());

        let calls = obs.llm_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].error.is_some());
    }

    // ── Default Observer methods ─────────────────────────────────────────────

    #[tokio::test]
    async fn default_observer_methods_are_no_ops() {
        struct NullObserver;
        #[async_trait]
        impl Observer for NullObserver {}

        let obs = NullObserver;
        let step = StepRecord {
            case_key: "k".into(),
            workflow_code: "wf".into(),
            step_name: "s".into(),
            input_resource_data: None,
            output: None,
            duration_ms: 0,
            timestamp: Utc::now(),
            cached: false,
            error: None,
        };
        obs.on_step(&step).await; // must not panic
        obs.flush().await;
    }
}
