// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

/// Observer integration tests for WorkflowContext and SchedulerV2.
///
/// TDD: these tests were written before the implementation. They describe
/// the expected behavior of the observer hooks.
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;
use workflow_engine::{
    BaseWorkflow, Case, InMemoryCaseStore, InMemoryStateStore, Observer, SchedulerEnvironment,
    SchedulerV2, StepRecord, WorkflowContext, WorkflowRecord, WorkflowRegistry, WorkflowResult,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

#[derive(Default)]
struct RecordingObserver {
    steps: Mutex<Vec<StepRecord>>,
    workflows: Mutex<Vec<WorkflowRecord>>,
    flushed: Mutex<bool>,
}

impl RecordingObserver {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl Observer for RecordingObserver {
    async fn on_step(&self, record: &StepRecord) {
        self.steps.lock().unwrap().push(record.clone());
    }
    async fn on_workflow_complete(&self, record: &WorkflowRecord) {
        self.workflows.lock().unwrap().push(record.clone());
    }
    async fn flush(&self) {
        *self.flushed.lock().unwrap() = true;
    }
}

fn make_ctx(key: &str) -> WorkflowContext {
    WorkflowContext::new(
        Case::new(key.into(), "sess_test".into(), "wf_test".into()),
        Arc::new(InMemoryCaseStore::default()),
        Arc::new(InMemoryStateStore::default()),
    )
}

fn make_ctx_with_observer(key: &str, observer: Arc<dyn Observer>) -> WorkflowContext {
    let mut ctx = make_ctx(key);
    ctx.set_observer(observer);
    ctx
}

// ── WorkflowContext: step() observer tests ───────────────────────────────────

#[tokio::test]
async fn on_step_called_for_fresh_execution() {
    let obs = RecordingObserver::new();
    let mut ctx = make_ctx_with_observer("case_fresh", obs.clone());

    ctx.step("greet", |_| async {
        Ok::<_, anyhow::Error>("hello".to_string())
    })
    .await
    .unwrap();

    let steps = obs.steps.lock().unwrap();
    assert_eq!(steps.len(), 1);
    let s = &steps[0];
    assert_eq!(s.step_name, "greet");
    assert_eq!(s.case_key, "case_fresh");
    assert!(!s.cached);
    assert!(s.error.is_none());
    assert_eq!(s.output, Some(json!("hello")));
}

#[tokio::test]
async fn on_step_called_with_cached_true_on_replay() {
    let obs = RecordingObserver::new();
    let mut ctx = make_ctx_with_observer("case_replay", obs.clone());

    // First execution: fresh
    ctx.step("greet", |_| async {
        Ok::<_, anyhow::Error>("hello".to_string())
    })
    .await
    .unwrap();

    // Second execution of the same step: should hit cache
    ctx.step("greet", |_| async {
        Ok::<_, anyhow::Error>("world".to_string())
    })
    .await
    .unwrap();

    let steps = obs.steps.lock().unwrap();
    assert_eq!(steps.len(), 2);
    assert!(!steps[0].cached);
    assert!(steps[1].cached);
    // Cached step should return original value, not "world"
    assert_eq!(steps[1].output, Some(json!("hello")));
}

#[tokio::test]
async fn on_step_records_error() {
    let obs = RecordingObserver::new();
    let mut ctx = make_ctx_with_observer("case_err", obs.clone());

    let result: anyhow::Result<String> = ctx
        .step("fail_step", |_| async {
            Err::<String, _>(anyhow::anyhow!("boom"))
        })
        .await;
    assert!(result.is_err());

    let steps = obs.steps.lock().unwrap();
    assert_eq!(steps.len(), 1);
    let s = &steps[0];
    assert_eq!(s.step_name, "fail_step");
    assert!(!s.cached);
    assert!(s.output.is_none());
    assert_eq!(s.error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn step_records_captures_input_resource_data() {
    let obs = RecordingObserver::new();
    let mut ctx = make_ctx_with_observer("case_input", obs.clone());
    ctx.case.resource_data = Some(json!({"doc_id": "abc123"}));

    ctx.step("process", |_| async { Ok::<_, anyhow::Error>(42u32) })
        .await
        .unwrap();

    let steps = obs.steps.lock().unwrap();
    assert_eq!(
        steps[0].input_resource_data,
        Some(json!({"doc_id": "abc123"}))
    );
}

#[tokio::test]
async fn step_records_accessible_via_accessor() {
    let mut ctx = make_ctx("case_accessor");
    ctx.step("s1", |_| async { Ok::<_, anyhow::Error>(1u32) })
        .await
        .unwrap();
    ctx.step("s2", |_| async { Ok::<_, anyhow::Error>(2u32) })
        .await
        .unwrap();

    let records = ctx.step_records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].step_name, "s1");
    assert_eq!(records[1].step_name, "s2");
}

#[tokio::test]
async fn no_observer_path_behaves_identically() {
    let mut ctx = make_ctx("case_no_obs");

    // No observer — should work exactly like before
    let result = ctx
        .step("plain", |_| async {
            Ok::<_, anyhow::Error>("unchanged".to_string())
        })
        .await
        .unwrap();
    assert_eq!(result, "unchanged");

    let result2 = ctx
        .step("plain", |_| async {
            Ok::<_, anyhow::Error>("cached_value_not_used".to_string())
        })
        .await
        .unwrap();
    assert_eq!(result2, "unchanged"); // still returns original
}

// ── WorkflowContext: finish() observer tests ─────────────────────────────────

#[tokio::test]
async fn on_workflow_complete_fires_on_finish() {
    let obs = RecordingObserver::new();
    let mut ctx = make_ctx_with_observer("case_finish", obs.clone());
    ctx.case.resource_data = Some(json!({"input": "doc"}));

    ctx.step("step_a", |_| async {
        Ok::<_, anyhow::Error>("a".to_string())
    })
    .await
    .unwrap();
    ctx.step("step_b", |_| async {
        Ok::<_, anyhow::Error>("b".to_string())
    })
    .await
    .unwrap();
    ctx.finish("success".into(), "done".into()).await.unwrap();

    let workflows = obs.workflows.lock().unwrap();
    assert_eq!(workflows.len(), 1);
    let w = &workflows[0];
    assert_eq!(w.case_key, "case_finish");
    assert_eq!(w.workflow_code, "wf_test");
    assert_eq!(w.finished_type.as_deref(), Some("success"));
    assert_eq!(w.steps.len(), 2);
    assert_eq!(w.steps[0].step_name, "step_a");
    assert_eq!(w.steps[1].step_name, "step_b");
}

#[tokio::test]
async fn flush_called_on_finish() {
    let obs = RecordingObserver::new();
    let mut ctx = make_ctx_with_observer("case_flush", obs.clone());
    ctx.finish("ok".into(), "done".into()).await.unwrap();

    assert!(*obs.flushed.lock().unwrap());
}

#[tokio::test]
async fn workflow_record_has_input_and_output_resource_data() {
    let obs = RecordingObserver::new();
    let mut ctx = make_ctx_with_observer("case_io", obs.clone());
    ctx.case.resource_data = Some(json!({"stage": "initial"}));

    // Simulate updating resource_data mid-workflow
    ctx.case.resource_data = Some(json!({"stage": "final"}));
    ctx.finish("done".into(), "desc".into()).await.unwrap();

    let wf = obs.workflows.lock().unwrap();
    // output_resource_data should reflect state at finish() time
    assert_eq!(wf[0].output_resource_data, Some(json!({"stage": "final"})));
}

#[tokio::test]
async fn step_records_cleared_after_finish() {
    let mut ctx = make_ctx("case_clear");
    ctx.step("s1", |_| async { Ok::<_, anyhow::Error>(1u32) })
        .await
        .unwrap();
    ctx.finish("ok".into(), "done".into()).await.unwrap();

    assert!(ctx.step_records().is_empty());
}

// ── SchedulerV2 observer injection ───────────────────────────────────────────

struct FinishWorkflowV2;
#[async_trait]
impl BaseWorkflow for FinishWorkflowV2 {
    async fn run(&self, ctx: &mut WorkflowContext) -> anyhow::Result<WorkflowResult> {
        ctx.step("compute", |_| async { Ok::<_, anyhow::Error>(42u32) })
            .await?;
        ctx.finish("SUCCESS".into(), "done".into()).await?;
        Ok(WorkflowResult::Finished("SUCCESS".into(), "done".into()))
    }
}

#[tokio::test]
async fn scheduler_injects_observer_into_contexts() {
    let obs = RecordingObserver::new();
    let mut scheduler = SchedulerV2::new();
    scheduler.set_observer(obs.clone() as Arc<dyn Observer>);

    let case = Case::new("case_sched".into(), "sess".into(), "finish_wf".into());
    let mut env = SchedulerEnvironment::from_session_id("sess", vec![case]);

    let mut registry = WorkflowRegistry::new();
    registry.register("finish_wf", |_| Box::new(FinishWorkflowV2));

    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());

    scheduler
        .tick(&mut env, &registry, cs, ss, None, None)
        .await
        .unwrap();

    let workflows = obs.workflows.lock().unwrap();
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].case_key, "case_sched");
}

#[tokio::test]
async fn scheduler_without_observer_still_works() {
    use workflow_engine::{SchedulerEnvironment, WorkflowRegistry};

    let mut scheduler = SchedulerV2::new();
    let case = Case::new("case_no_obs".into(), "sess".into(), "finish_wf".into());
    let mut env = SchedulerEnvironment::from_session_id("sess", vec![case]);

    let mut registry = WorkflowRegistry::new();
    registry.register("finish_wf", |_| Box::new(FinishWorkflowV2));

    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());

    let result = scheduler
        .tick(&mut env, &registry, cs, ss, None, None)
        .await
        .unwrap();
    assert!(result.any_executed());
}