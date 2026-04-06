// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

/// Tests for CaseStore, StateStore traits and their InMemory implementations.
/// These run without any database — pure in-memory validation.
use tianshu::{
    case::{Case, ExecutionState},
    store::{CaseStore, InMemoryCaseStore, InMemoryStateStore, StateStore},
};

// ── CaseStore (InMemory) ─────────────────────────────────────────────────────

#[tokio::test]
async fn case_store_upsert_and_get_by_key() {
    let store = InMemoryCaseStore::default();
    let case = Case::new("key1".into(), "sess1".into(), "wf_a".into());

    store.upsert(&case).await.unwrap();
    let fetched = store.get_by_key("key1").await.unwrap();

    assert!(fetched.is_some());
    let c = fetched.unwrap();
    assert_eq!(c.case_key, "key1");
    assert_eq!(c.workflow_code, "wf_a");
    assert_eq!(c.execution_state, ExecutionState::Running);
}

#[tokio::test]
async fn case_store_get_missing_returns_none() {
    let store = InMemoryCaseStore::default();
    let result = store.get_by_key("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn case_store_upsert_updates_existing() {
    let store = InMemoryCaseStore::default();
    let mut case = Case::new("key1".into(), "sess1".into(), "wf_a".into());
    store.upsert(&case).await.unwrap();

    case.mc_finish("success".into(), "done".into());
    store.upsert(&case).await.unwrap();

    let fetched = store.get_by_key("key1").await.unwrap().unwrap();
    assert_eq!(fetched.execution_state, ExecutionState::Finished);
}

#[tokio::test]
async fn case_store_get_by_session_returns_all_matching() {
    let store = InMemoryCaseStore::default();

    store
        .upsert(&Case::new("k1".into(), "sess_A".into(), "wf".into()))
        .await
        .unwrap();
    store
        .upsert(&Case::new("k2".into(), "sess_A".into(), "wf".into()))
        .await
        .unwrap();
    store
        .upsert(&Case::new("k3".into(), "sess_B".into(), "wf".into()))
        .await
        .unwrap();

    let cases = store.get_by_session("sess_A").await.unwrap();
    assert_eq!(cases.len(), 2);
    let keys: Vec<_> = cases.iter().map(|c| c.case_key.as_str()).collect();
    assert!(keys.contains(&"k1"));
    assert!(keys.contains(&"k2"));
}

#[tokio::test]
async fn case_store_get_by_session_empty() {
    let store = InMemoryCaseStore::default();
    let cases = store.get_by_session("no_such_session").await.unwrap();
    assert!(cases.is_empty());
}

// ── StateStore (InMemory) ────────────────────────────────────────────────────

#[tokio::test]
async fn state_store_save_and_get() {
    let store = InMemoryStateStore::default();
    store
        .save("case1", "step_init", r#"{"result":42}"#)
        .await
        .unwrap();

    let entry = store.get("case1", "step_init").await.unwrap();
    assert!(entry.is_some());
    let e = entry.unwrap();
    assert_eq!(e.case_key, "case1");
    assert_eq!(e.step, "step_init");
    assert_eq!(e.data, r#"{"result":42}"#);
}

#[tokio::test]
async fn state_store_get_missing_returns_none() {
    let store = InMemoryStateStore::default();
    let result = store.get("no_case", "no_step").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn state_store_save_overwrites_existing() {
    let store = InMemoryStateStore::default();
    store.save("c1", "step1", r#"{"v":1}"#).await.unwrap();
    store.save("c1", "step1", r#"{"v":99}"#).await.unwrap();

    let entry = store.get("c1", "step1").await.unwrap().unwrap();
    assert_eq!(entry.data, r#"{"v":99}"#);
}

#[tokio::test]
async fn state_store_get_all_returns_all_steps_for_case() {
    let store = InMemoryStateStore::default();
    store.save("case1", "step_a", "data_a").await.unwrap();
    store.save("case1", "step_b", "data_b").await.unwrap();
    store.save("case2", "step_a", "other").await.unwrap();

    let entries = store.get_all("case1").await.unwrap();
    assert_eq!(entries.len(), 2);
    let steps: Vec<_> = entries.iter().map(|e| e.step.as_str()).collect();
    assert!(steps.contains(&"step_a"));
    assert!(steps.contains(&"step_b"));
}

#[tokio::test]
async fn state_store_delete_by_case_removes_all_steps() {
    let store = InMemoryStateStore::default();
    store.save("case1", "step_a", "x").await.unwrap();
    store.save("case1", "step_b", "y").await.unwrap();
    store.save("case2", "step_a", "z").await.unwrap();

    store.delete_by_case("case1").await.unwrap();

    assert!(store.get_all("case1").await.unwrap().is_empty());
    // case2 is untouched
    assert_eq!(store.get_all("case2").await.unwrap().len(), 1);
}

#[tokio::test]
async fn state_store_delete_by_case_idempotent() {
    let store = InMemoryStateStore::default();
    // Deleting a non-existent case should not error
    store.delete_by_case("ghost").await.unwrap();
}
