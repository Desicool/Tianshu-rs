// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

/// PostgreSQL store integration tests.
///
/// These tests require a live PostgreSQL instance and are gated with `#[ignore]`.
/// To run: set DATABASE_URL env var and pass `-- --ignored`.
///
/// Example:
///   DATABASE_URL=postgres://postgres:password@localhost/test_db \
///     cargo test -p workflow_engine_postgres --test postgres_store_tests -- --ignored
use std::sync::Arc;
use tianshu::{
    case::{Case, ExecutionState},
    store::{CaseStore, StateStore},
};
use tianshu_postgres::{PostgresCaseStore, PostgresStateStore};

async fn make_stores() -> (Arc<PostgresCaseStore>, Arc<PostgresStateStore>) {
    let db_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set to run postgres integration tests");
    let pool = tianshu_postgres::build_pool(&db_url).expect("failed to build connection pool");

    let case_store = Arc::new(PostgresCaseStore::new(pool.clone()));
    let state_store = Arc::new(PostgresStateStore::new(pool));

    case_store.setup().await.expect("case_store setup failed");
    state_store.setup().await.expect("state_store setup failed");

    (case_store, state_store)
}

#[tokio::test]
#[ignore]
async fn postgres_case_store_upsert_get() {
    let (cs, _) = make_stores().await;
    let key = format!("pg_test_{}", uuid::Uuid::new_v4());
    let case = Case::new(key.clone(), "pg_sess".into(), "wf_pg".into());

    cs.upsert(&case).await.unwrap();
    let fetched = cs.get_by_key(&key).await.unwrap().unwrap();
    assert_eq!(fetched.workflow_code, "wf_pg");
}

#[tokio::test]
#[ignore]
async fn postgres_case_store_upsert_updates() {
    let (cs, _) = make_stores().await;
    let key = format!("pg_upd_{}", uuid::Uuid::new_v4());
    let mut case = Case::new(key.clone(), "pg_sess".into(), "wf".into());
    cs.upsert(&case).await.unwrap();

    case.mc_finish("ok".into(), "done".into());
    cs.upsert(&case).await.unwrap();

    let fetched = cs.get_by_key(&key).await.unwrap().unwrap();
    assert_eq!(fetched.execution_state, ExecutionState::Finished);
}

#[tokio::test]
#[ignore]
async fn postgres_case_store_get_by_session() {
    let (cs, _) = make_stores().await;
    let sess = format!("pg_sess_{}", uuid::Uuid::new_v4());

    for i in 0..3 {
        cs.upsert(&Case::new(
            format!("k_{}_{}", sess, i),
            sess.clone(),
            "wf".into(),
        ))
        .await
        .unwrap();
    }

    let cases = cs.get_by_session(&sess).await.unwrap();
    assert_eq!(cases.len(), 3);
}

#[tokio::test]
#[ignore]
async fn postgres_state_store_save_get_delete() {
    let (_, ss) = make_stores().await;
    let case_key = format!("pg_state_{}", uuid::Uuid::new_v4());

    ss.save(&case_key, "step_a", r#"{"v":1}"#).await.unwrap();
    ss.save(&case_key, "step_b", r#"{"v":2}"#).await.unwrap();

    let e = ss.get(&case_key, "step_a").await.unwrap().unwrap();
    assert_eq!(e.data, r#"{"v":1}"#);

    let all = ss.get_all(&case_key).await.unwrap();
    assert_eq!(all.len(), 2);

    ss.delete_by_case(&case_key).await.unwrap();
    assert!(ss.get_all(&case_key).await.unwrap().is_empty());
}
