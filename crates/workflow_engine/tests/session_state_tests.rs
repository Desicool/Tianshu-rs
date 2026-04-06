// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use workflow_engine::store::{InMemoryStateStore, StateStore};

#[tokio::test]
async fn session_state_save_and_get() {
    let store = InMemoryStateStore::default();
    store
        .save_session("sess_1", "step_a", r#""hello""#)
        .await
        .unwrap();

    let entry = store.get_session("sess_1", "step_a").await.unwrap();
    assert!(entry.is_some());
    let entry = entry.unwrap();
    assert_eq!(entry.session_id, "sess_1");
    assert_eq!(entry.step, "step_a");
    assert_eq!(entry.data, r#""hello""#);
}

#[tokio::test]
async fn session_state_get_missing_returns_none() {
    let store = InMemoryStateStore::default();
    let result = store.get_session("sess_1", "missing").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn session_state_save_overwrites() {
    let store = InMemoryStateStore::default();
    store
        .save_session("sess_1", "step_a", r#""v1""#)
        .await
        .unwrap();
    store
        .save_session("sess_1", "step_a", r#""v2""#)
        .await
        .unwrap();

    let entry = store
        .get_session("sess_1", "step_a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.data, r#""v2""#);
}

#[tokio::test]
async fn session_state_get_all_returns_all_steps() {
    let store = InMemoryStateStore::default();
    store.save_session("sess_1", "step_a", "a").await.unwrap();
    store.save_session("sess_1", "step_b", "b").await.unwrap();
    store.save_session("sess_2", "step_c", "c").await.unwrap();

    let entries = store.get_all_session("sess_1").await.unwrap();
    assert_eq!(entries.len(), 2);
    let steps: Vec<_> = entries.iter().map(|e| e.step.as_str()).collect();
    assert!(steps.contains(&"step_a"));
    assert!(steps.contains(&"step_b"));
}

#[tokio::test]
async fn session_state_delete_by_session() {
    let store = InMemoryStateStore::default();
    store.save_session("sess_1", "step_a", "a").await.unwrap();
    store.save_session("sess_1", "step_b", "b").await.unwrap();
    store.save_session("sess_2", "step_c", "c").await.unwrap();

    store.delete_by_session("sess_1").await.unwrap();

    assert!(store.get_all_session("sess_1").await.unwrap().is_empty());
    // sess_2 untouched
    assert_eq!(store.get_all_session("sess_2").await.unwrap().len(), 1);
}

#[tokio::test]
async fn session_state_delete_by_session_idempotent() {
    let store = InMemoryStateStore::default();
    store.delete_by_session("nonexistent").await.unwrap();
}

#[tokio::test]
async fn session_state_isolated_from_case_state() {
    let store = InMemoryStateStore::default();

    // Save case-scoped state
    store
        .save("case_1", "shared_key", "case_val")
        .await
        .unwrap();
    // Save session-scoped state with same step name
    store
        .save_session("sess_1", "shared_key", "session_val")
        .await
        .unwrap();

    // Case state should not see session state
    let case_entries = store.get_all("case_1").await.unwrap();
    assert_eq!(case_entries.len(), 1);
    assert_eq!(case_entries[0].data, "case_val");

    // Session state should not see case state
    let session_entries = store.get_all_session("sess_1").await.unwrap();
    assert_eq!(session_entries.len(), 1);
    assert_eq!(session_entries[0].data, "session_val");
}