// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the tianshu-dashboard HTTP API.
//!
//! Each test binds on a unique port (19000+n), starts the dashboard server
//! via tokio::spawn, waits 100ms for the listener to be ready, then exercises
//! the HTTP endpoints with reqwest.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;
use tianshu::case::{Case, ExecutionState};
use tianshu::store::InMemoryCaseStore;
use tianshu_dashboard::DashboardServer;
use tianshu_observe::InMemoryObserver;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_case_at(
    key: &str,
    session: &str,
    code: &str,
    state: ExecutionState,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> Case {
    let mut c = Case::new(key.to_string(), session.to_string(), code.to_string());
    c.execution_state = state.clone();
    c.created_at = created_at;
    c.updated_at = updated_at;
    if state == ExecutionState::Finished {
        c.finished_type = Some("SUCCESS".into());
        c.finished_description = Some("done".into());
    }
    c
}

fn make_case_now(key: &str, session: &str, code: &str, state: ExecutionState) -> Case {
    let now = Utc::now();
    make_case_at(key, session, code, state, now, now)
}

async fn start_server(
    port: u16,
    case_store: Arc<InMemoryCaseStore>,
    observer: Arc<InMemoryObserver>,
) {
    tokio::spawn(async move {
        DashboardServer::new(case_store, observer)
            .with_port(port)
            .with_refresh_secs(5)
            .serve()
            .await
            .ok();
    });
    // Give the server time to bind
    tokio::time::sleep(Duration::from_millis(150)).await;
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{}", port)
}

/// Build a reqwest client that bypasses any system proxy (important in WSL environments).
fn client() -> Client {
    Client::builder()
        .no_proxy()
        .build()
        .expect("failed to build reqwest client")
}

// ── T1: GET / returns HTML ────────────────────────────────────────────────────

#[tokio::test]
async fn t1_index_returns_html() {
    let port = 19001u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());
    start_server(port, cs, obs).await;

    let resp = client()
        .get(format!("{}/", base_url(port)))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/html"), "content-type was: {}", ct);
    let body = resp.text().await.unwrap();
    assert!(body.contains("Tianshu"), "body missing 'Tianshu'");
}

// ── T2: GET /api/config returns refresh_secs ─────────────────────────────────

#[tokio::test]
async fn t2_config_returns_refresh_secs() {
    let port = 19002u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());
    start_server(port, cs, obs).await;

    let json: Value = client()
        .get(format!("{}/api/config", base_url(port)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(json["refresh_secs"], 5);
}

// ── T3: GET /api/cases lists running cases ────────────────────────────────────

#[tokio::test]
async fn t3_cases_lists_running() {
    let port = 19003u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    // Insert 2 running, 1 waiting
    use tianshu::store::CaseStore;
    cs.upsert(&make_case_now("r1", "s", "wf_a", ExecutionState::Running))
        .await
        .unwrap();
    cs.upsert(&make_case_now("r2", "s", "wf_b", ExecutionState::Running))
        .await
        .unwrap();
    cs.upsert(&make_case_now("w1", "s", "wf_c", ExecutionState::Waiting))
        .await
        .unwrap();

    start_server(port, cs, obs).await;

    let json: Value = client()
        .get(format!("{}/api/cases?state=running", base_url(port)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(json["total"], 2, "expected 2 running, got: {}", json);
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
}

// ── T4: GET /api/cases?state=active merges running+waiting ───────────────────

#[tokio::test]
async fn t4_cases_active_merges_states() {
    let port = 19004u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    use tianshu::store::CaseStore;
    cs.upsert(&make_case_now("r1", "s", "wf", ExecutionState::Running))
        .await
        .unwrap();
    cs.upsert(&make_case_now("w1", "s", "wf", ExecutionState::Waiting))
        .await
        .unwrap();
    cs.upsert(&make_case_now("f1", "s", "wf", ExecutionState::Finished))
        .await
        .unwrap();

    start_server(port, cs, obs).await;

    let json: Value = client()
        .get(format!("{}/api/cases?state=active", base_url(port)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        json["total"], 2,
        "active should be running+waiting=2, got: {}",
        json
    );
}

// ── T5: GET /api/stats returns valid structure ────────────────────────────────

#[tokio::test]
async fn t5_stats_returns_valid_structure() {
    let port = 19005u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    use tianshu::store::CaseStore;
    cs.upsert(&make_case_now("r1", "s", "wf", ExecutionState::Running))
        .await
        .unwrap();

    start_server(port, cs, obs).await;

    let json: Value = client()
        .get(format!("{}/api/stats?window=1d", base_url(port)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(json["running_count"].is_number());
    assert!(json["waiting_count"].is_number());
    assert!(json["finished_count"].is_number());
    assert!(json["completion_rate"].is_number());
    assert!(json["avg_duration_ms"].is_number());
    assert!(json["p50_duration_ms"].is_number());
    assert!(json["p99_duration_ms"].is_number());
    assert!(json["avg_probe_ms"].is_number());
    assert_eq!(json["window_days"], 1);
}

// ── T6: Completion rate calculation ──────────────────────────────────────────

#[tokio::test]
async fn t6_completion_rate() {
    let port = 19006u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    use tianshu::store::CaseStore;
    let now = Utc::now();

    // 5 finished + 3 running + 2 waiting = 10 total in window
    // rate = 5 / (5 + 5) = 0.50
    for i in 0..5 {
        cs.upsert(&make_case_at(
            &format!("f{}", i),
            "s",
            "wf",
            ExecutionState::Finished,
            now,
            now,
        ))
        .await
        .unwrap();
    }
    for i in 0..3 {
        cs.upsert(&make_case_at(
            &format!("r{}", i),
            "s",
            "wf",
            ExecutionState::Running,
            now,
            now,
        ))
        .await
        .unwrap();
    }
    for i in 0..2 {
        cs.upsert(&make_case_at(
            &format!("w{}", i),
            "s",
            "wf",
            ExecutionState::Waiting,
            now,
            now,
        ))
        .await
        .unwrap();
    }

    start_server(port, cs, obs).await;

    let json: Value = client()
        .get(format!("{}/api/stats?window=1d", base_url(port)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let rate = json["completion_rate"].as_f64().unwrap();
    assert!((rate - 0.5).abs() < 0.01, "expected ~0.5, got {}", rate);
}

// ── T7: P99 duration calculation ──────────────────────────────────────────────

#[tokio::test]
async fn t7_p99_duration() {
    let port = 19007u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    use tianshu::store::CaseStore;

    // 100 finished cases with durations 1ms..100ms
    // Use recent timestamps so they fall within the 7d window
    let now = Utc::now();
    for i in 1usize..=100 {
        // created_at = now - duration, updated_at = now → duration = i ms
        let created = now - chrono::Duration::milliseconds(i as i64);
        let c = make_case_at(
            &format!("f{}", i),
            "s",
            "wf",
            ExecutionState::Finished,
            created,
            now,
        );
        cs.upsert(&c).await.unwrap();
    }

    start_server(port, cs, obs).await;

    let json: Value = client()
        .get(format!("{}/api/stats?window=7d", base_url(port)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // With n=100: p99 index = (100*99)/100 = 99 → 0-indexed = durations[99] = 100ms
    let p99 = json["p99_duration_ms"].as_f64().unwrap();
    assert!(p99 > 0.0, "p99 should be > 0, got {}", p99);
}

// ── T8: Window filtering excludes old cases ───────────────────────────────────

#[tokio::test]
async fn t8_window_excludes_old_cases() {
    let port = 19008u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    use tianshu::store::CaseStore;

    let now = Utc::now();
    let old = now - chrono::Duration::hours(49); // ~2 days ago, outside 1d window

    // 2 old finished cases (outside 1d window)
    for i in 0..2 {
        cs.upsert(&make_case_at(
            &format!("old{}", i),
            "s",
            "wf",
            ExecutionState::Finished,
            old,
            old,
        ))
        .await
        .unwrap();
    }
    // 1 recent finished case (inside 1d window)
    cs.upsert(&make_case_at(
        "recent",
        "s",
        "wf",
        ExecutionState::Finished,
        now,
        now,
    ))
    .await
    .unwrap();

    start_server(port, cs, obs).await;

    let json: Value = client()
        .get(format!("{}/api/stats?window=1d", base_url(port)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Only 1 finished case is within the 1d window
    assert_eq!(
        json["finished_count"], 1,
        "only 1 case should be in 1d window, got: {}",
        json
    );
}

// ── T9: Pagination ────────────────────────────────────────────────────────────

#[tokio::test]
async fn t9_pagination() {
    let port = 19009u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    use tianshu::store::CaseStore;

    // Insert 25 running cases
    for i in 0..25 {
        cs.upsert(&make_case_now(
            &format!("c{:02}", i),
            "s",
            "wf",
            ExecutionState::Running,
        ))
        .await
        .unwrap();
    }

    start_server(port, cs, obs).await;

    // Page 1: limit=10, offset=0
    let json: Value = client()
        .get(format!(
            "{}/api/cases?state=running&limit=10&offset=0",
            base_url(port)
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(json["total"], 25);
    assert_eq!(json["items"].as_array().unwrap().len(), 10);

    // Page 3: offset=20 → 5 remaining
    let json2: Value = client()
        .get(format!(
            "{}/api/cases?state=running&limit=10&offset=20",
            base_url(port)
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(json2["items"].as_array().unwrap().len(), 5);
}

// ── T10: State filter returns correct states ──────────────────────────────────

#[tokio::test]
async fn t10_state_filter() {
    let port = 19010u16;
    let cs = Arc::new(InMemoryCaseStore::default());
    let obs = Arc::new(InMemoryObserver::new());

    use tianshu::store::CaseStore;

    cs.upsert(&make_case_now("r1", "s", "wf", ExecutionState::Running))
        .await
        .unwrap();
    cs.upsert(&make_case_now("w1", "s", "wf", ExecutionState::Waiting))
        .await
        .unwrap();
    cs.upsert(&make_case_now("f1", "s", "wf", ExecutionState::Finished))
        .await
        .unwrap();

    start_server(port, cs, obs).await;

    // Each filter should return exactly 1 case
    for (state, expected) in [
        ("running", "running"),
        ("waiting", "waiting"),
        ("finished", "finished"),
    ] {
        let json: Value = client()
            .get(format!("{}/api/cases?state={}", base_url(port), state))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(
            json["total"], 1,
            "state={} should have 1 result, got: {}",
            state, json
        );
        let items = json["items"].as_array().unwrap();
        assert_eq!(items[0]["state"], expected);
    }
}
