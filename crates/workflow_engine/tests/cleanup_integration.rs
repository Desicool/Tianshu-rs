/// Integration tests for WorkflowContext cleanup behaviour.
///
/// These tests verify that:
/// - State entries are removed when a workflow finishes
/// - Case status is persisted after finish
/// - Multiple cases in the same session don't interfere
///
/// All tests use InMemoryStateStore / InMemoryCaseStore — no database required.
use std::sync::Arc;
use workflow_engine::{
    case::{Case, ExecutionState},
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
    WorkflowContext,
};

fn make_ctx(
    case_key: &str,
    session_id: &str,
) -> (
    WorkflowContext,
    Arc<InMemoryCaseStore>,
    Arc<InMemoryStateStore>,
) {
    let case = Case::new(case_key.into(), session_id.into(), "test_wf".into());
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let ctx = WorkflowContext::new(
        case,
        Arc::clone(&case_store) as Arc<dyn CaseStore>,
        Arc::clone(&state_store) as Arc<dyn StateStore>,
    );
    (ctx, case_store, state_store)
}

#[tokio::test]
async fn finish_clears_all_state_entries() {
    let (mut ctx, _cs, state_store) = make_ctx("case_cleanup_1", "sess_1");

    ctx.save_checkpoint("step_a", serde_json::json!(1))
        .await
        .unwrap();
    ctx.save_checkpoint("step_b", serde_json::json!(2))
        .await
        .unwrap();

    // State exists before finish
    assert_eq!(
        state_store.get_all("case_cleanup_1").await.unwrap().len(),
        2
    );

    ctx.finish("success".into(), "done".into()).await.unwrap();

    // State should be cleared
    assert!(state_store
        .get_all("case_cleanup_1")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn finish_persists_case_status() {
    let (mut ctx, case_store, _ss) = make_ctx("case_fin_status", "sess_1");

    ctx.finish("abnormal".into(), "timeout".into())
        .await
        .unwrap();

    let stored = case_store
        .get_by_key("case_fin_status")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.execution_state, ExecutionState::Finished);
    assert_eq!(stored.finished_type.as_deref(), Some("abnormal"));
    assert_eq!(stored.finished_description.as_deref(), Some("timeout"));
}

#[tokio::test]
async fn finish_does_not_affect_other_cases_state() {
    // Case A
    let (mut ctx_a, _cs_a, state_store_a) = make_ctx("case_A", "shared_sess");
    ctx_a
        .save_checkpoint("step1", serde_json::json!("a"))
        .await
        .unwrap();

    // Case B uses its own isolated stores
    let case_b = Case::new("case_B".into(), "shared_sess".into(), "wf".into());
    let state_store_b = Arc::new(InMemoryStateStore::default());
    let mut ctx_b = WorkflowContext::new(
        case_b,
        Arc::new(InMemoryCaseStore::default()) as Arc<dyn CaseStore>,
        Arc::clone(&state_store_b) as Arc<dyn StateStore>,
    );
    ctx_b
        .save_checkpoint("step1", serde_json::json!("b"))
        .await
        .unwrap();

    // Finish A only
    ctx_a.finish("ok".into(), "done".into()).await.unwrap();

    // A's state is gone
    assert!(state_store_a.get_all("case_A").await.unwrap().is_empty());
    // B's state is untouched
    assert_eq!(state_store_b.get_all("case_B").await.unwrap().len(), 1);
}

#[tokio::test]
async fn clear_step_removes_single_checkpoint() {
    let (mut ctx, _cs, _ss) = make_ctx("case_clear_step", "sess_1");

    ctx.save_checkpoint("step_x", serde_json::json!(10))
        .await
        .unwrap();
    ctx.save_checkpoint("step_y", serde_json::json!(20))
        .await
        .unwrap();

    ctx.clear_step("step_x").await.unwrap();

    // step_x should be null after clear
    let ck = ctx.get_checkpoint("step_x").await.unwrap();
    assert!(ck.is_none() || ck.unwrap().is_null());

    // step_y is untouched
    let ky = ctx.get_checkpoint("step_y").await.unwrap();
    assert_eq!(ky.unwrap(), serde_json::json!(20));
}

#[tokio::test]
async fn step_is_idempotent_on_second_call() {
    let (mut ctx, _cs, _ss) = make_ctx("case_step_idem", "sess_1");

    let mut exec_count = 0_u32;

    let result: u32 = ctx
        .step("compute", |_| {
            exec_count += 1;
            async move { Ok(42_u32) }
        })
        .await
        .unwrap();
    assert_eq!(result, 42);
    assert_eq!(exec_count, 1);

    // Second call restores from checkpoint — fn must NOT be called again
    let result2: u32 = ctx
        .step("compute", |_| {
            exec_count += 1;
            async move { Ok(99_u32) }
        })
        .await
        .unwrap();
    assert_eq!(result2, 42); // restored, not 99
    assert_eq!(exec_count, 1); // fn not called a second time
}
