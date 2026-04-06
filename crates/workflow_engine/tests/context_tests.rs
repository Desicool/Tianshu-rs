// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

/// Tests for WorkflowContext with trait-based CaseStore + StateStore.
use std::sync::Arc;
use tianshu::{
    case::{Case, ExecutionState},
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
    WorkflowContext,
};

fn make_case(key: &str) -> Case {
    Case::new(key.into(), "sess1".into(), "wf_test".into())
}

#[tokio::test]
async fn context_save_and_restore_checkpoint() {
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_ck_1");

    let mut ctx = WorkflowContext::new(case, case_store, state_store);

    ctx.save_checkpoint("step1", serde_json::json!({"val": 42}))
        .await
        .unwrap();

    let restored = ctx.get_checkpoint("step1").await.unwrap();
    assert!(restored.is_some());
    assert_eq!(restored.unwrap()["val"], 42);
}

#[tokio::test]
async fn context_checkpoint_cached_after_first_load() {
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_ck_2");
    let mut ctx = WorkflowContext::new(case, case_store, state_store);

    ctx.save_checkpoint("step_x", serde_json::json!("hello"))
        .await
        .unwrap();

    // First get loads from store
    let v1 = ctx.get_checkpoint("step_x").await.unwrap();
    // Second get should hit cache (no additional store call observable from outside,
    // but the value must be the same)
    let v2 = ctx.get_checkpoint("step_x").await.unwrap();
    assert_eq!(v1, v2);
}

#[tokio::test]
async fn context_get_checkpoint_returns_none_for_missing() {
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_ck_3");
    let mut ctx = WorkflowContext::new(case, case_store, state_store);

    let result = ctx.get_checkpoint("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn context_clear_step_removes_checkpoint() {
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_ck_4");
    let mut ctx = WorkflowContext::new(case, case_store, state_store);

    ctx.save_checkpoint("step_z", serde_json::json!(true))
        .await
        .unwrap();
    ctx.clear_step("step_z").await.unwrap();

    let result = ctx.get_checkpoint("step_z").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn context_finish_marks_case_and_cleans_state() {
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_fin_1");
    let mut ctx = WorkflowContext::new(case, case_store.clone(), state_store.clone());

    ctx.save_checkpoint("step_a", serde_json::json!(1))
        .await
        .unwrap();
    ctx.save_checkpoint("step_b", serde_json::json!(2))
        .await
        .unwrap();

    ctx.finish("success".into(), "all done".into())
        .await
        .unwrap();

    // Case should be marked finished in the store
    let stored = case_store.get_by_key("case_fin_1").await.unwrap().unwrap();
    assert_eq!(stored.execution_state, ExecutionState::Finished);
    assert_eq!(stored.finished_type.as_deref(), Some("success"));

    // State should be cleaned up
    let state_entries: Vec<_> = state_store.get_all("case_fin_1").await.unwrap();
    assert!(state_entries.is_empty());
}

#[tokio::test]
async fn context_step_executes_and_checkpoints() {
    let case_store = Arc::new(InMemoryCaseStore::default());
    let state_store = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_step_1");
    let mut ctx = WorkflowContext::new(case, case_store, state_store);

    let result: i32 = ctx
        .step("compute", |_ctx| async move { Ok(7_i32) })
        .await
        .unwrap();

    assert_eq!(result, 7);

    // Second call must restore from checkpoint without re-executing
    let result2: i32 = ctx
        .step(
            "compute",
            |_ctx| async move { panic!("Should not re-execute") },
        )
        .await
        .unwrap();

    assert_eq!(result2, 7);
}

// ── Session-scoped state tests ──────────────────────────────────────────────

#[tokio::test]
async fn context_set_and_get_session_state() {
    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_ss_1");
    let mut ctx = WorkflowContext::new(case, cs, ss);

    ctx.set_session_state("counter", 42_i32).await.unwrap();
    let val: i32 = ctx.get_session_state("counter", 0).await.unwrap();
    assert_eq!(val, 42);
}

#[tokio::test]
async fn context_get_session_state_returns_default_when_unset() {
    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_ss_2");
    let mut ctx = WorkflowContext::new(case, cs, ss);

    let val: String = ctx
        .get_session_state("missing", "default_val".to_string())
        .await
        .unwrap();
    assert_eq!(val, "default_val");
}

#[tokio::test]
async fn context_two_cases_share_session_state() {
    let cs = Arc::new(InMemoryCaseStore::default());
    let ss: Arc<InMemoryStateStore> = Arc::new(InMemoryStateStore::default());

    // Two cases in the same session
    let case_a = Case::new("case_a".into(), "shared_sess".into(), "wf".into());
    let case_b = Case::new("case_b".into(), "shared_sess".into(), "wf".into());

    let mut ctx_a = WorkflowContext::new(case_a, cs.clone(), ss.clone());
    let mut ctx_b = WorkflowContext::new(case_b, cs, ss);

    // Case A writes session state
    ctx_a
        .set_session_state("shared_var", "from_a".to_string())
        .await
        .unwrap();

    // Case B reads the same session state
    let val: String = ctx_b
        .get_session_state("shared_var", "none".to_string())
        .await
        .unwrap();
    assert_eq!(val, "from_a");
}

#[tokio::test]
async fn context_session_state_independent_of_case_state() {
    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());
    let case = make_case("case_ss_4");
    let mut ctx = WorkflowContext::new(case, cs, ss);

    // Set case-scoped and session-scoped state with the same name
    ctx.set_state("x", "case_value".to_string()).await.unwrap();
    ctx.set_session_state("x", "session_value".to_string())
        .await
        .unwrap();

    // They should be independent
    let case_val: String = ctx.get_state("x", "none".to_string()).await.unwrap();
    let session_val: String = ctx
        .get_session_state("x", "none".to_string())
        .await
        .unwrap();

    assert_eq!(case_val, "case_value");
    assert_eq!(session_val, "session_value");
}
