// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use tianshu::session::Session;
use tianshu::store::{InMemorySessionStore, SessionStore};

#[test]
fn session_new_sets_defaults() {
    let session = Session::new("sess_1");
    assert_eq!(session.session_id, "sess_1");
    assert!(session.metadata.is_none());
    assert!(session.created_at <= session.updated_at);
}

#[test]
fn session_with_metadata() {
    let meta = serde_json::json!({"user": "alice", "channel": "web"});
    let session = Session::new("sess_2").with_metadata(meta.clone());
    assert_eq!(session.metadata, Some(meta));
}

#[tokio::test]
async fn session_store_upsert_and_get() {
    let store = InMemorySessionStore::default();
    let session = Session::new("sess_a");
    store.upsert(&session).await.unwrap();

    let fetched = store.get("sess_a").await.unwrap();
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().session_id, "sess_a");
}

#[tokio::test]
async fn session_store_get_missing_returns_none() {
    let store = InMemorySessionStore::default();
    let result = store.get("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn session_store_upsert_updates_existing() {
    let store = InMemorySessionStore::default();

    let session = Session::new("sess_b");
    store.upsert(&session).await.unwrap();

    let updated = Session::new("sess_b").with_metadata(serde_json::json!({"updated": true}));
    store.upsert(&updated).await.unwrap();

    let fetched = store.get("sess_b").await.unwrap().unwrap();
    assert!(fetched.metadata.is_some());
    assert_eq!(fetched.metadata.unwrap()["updated"], true);
}

#[tokio::test]
async fn session_store_delete() {
    let store = InMemorySessionStore::default();
    store.upsert(&Session::new("sess_c")).await.unwrap();
    store.delete("sess_c").await.unwrap();
    assert!(store.get("sess_c").await.unwrap().is_none());
}

#[tokio::test]
async fn session_store_delete_idempotent() {
    let store = InMemorySessionStore::default();
    // Deleting a nonexistent session should not error
    store.delete("nonexistent").await.unwrap();
}
