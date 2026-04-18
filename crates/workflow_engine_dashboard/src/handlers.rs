// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use tianshu::case::ExecutionState;
use tianshu::store::CaseFilter;

use crate::metrics::compute_stats;
use crate::server::AppState;
use crate::types::CaseRow;

// ── GET / ─────────────────────────────────────────────────────────────────────

/// Serve the dashboard HTML.
pub async fn get_index() -> Response {
    let html = include_str!("assets/index.html");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

// ── GET /api/config ───────────────────────────────────────────────────────────

/// Return dashboard configuration (refresh interval).
pub async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "refresh_secs": state.refresh_secs }))
}

// ── GET /api/cases ────────────────────────────────────────────────────────────

/// Query parameters for the `/api/cases` endpoint.
#[derive(Debug, Deserialize)]
pub struct CasesParams {
    /// Filter by state: "running", "waiting", "finished", or "active" (running+waiting).
    pub state: Option<String>,
    /// Maximum number of results (default 20, max 100).
    pub limit: Option<usize>,
    /// Number of results to skip (default 0).
    pub offset: Option<usize>,
}

/// Response body for `/api/cases`.
#[derive(Debug, Serialize)]
struct CasesResponse {
    items: Vec<CaseRow>,
    total: usize,
}

/// List cases with optional filtering and pagination.
pub async fn get_cases(
    State(state): State<AppState>,
    Query(params): Query<CasesParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);
    let now = Utc::now();

    let cases = match params.state.as_deref() {
        Some("active") => {
            // Merge Running + Waiting, sort by created_at desc, then paginate
            let filter_running = CaseFilter {
                execution_state: Some(ExecutionState::Running),
                ..Default::default()
            };
            let filter_waiting = CaseFilter {
                execution_state: Some(ExecutionState::Waiting),
                ..Default::default()
            };
            let mut running = state
                .case_store
                .list(filter_running)
                .await
                .unwrap_or_default();
            let waiting = state
                .case_store
                .list(filter_waiting)
                .await
                .unwrap_or_default();
            running.extend(waiting);
            running.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            let total = running.len();
            let items: Vec<CaseRow> = running
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|c| CaseRow {
                    age_secs: (now - c.created_at).num_seconds(),
                    workflow_code: c.workflow_code.clone(),
                    state: c.execution_state.to_string(),
                    created_at: c.created_at,
                    session_id: c.session_id.clone(),
                    parent_key: c.parent_key.clone(),
                    child_count: c.child_keys.len(),
                    resource_data: c.resource_data.clone(),
                    case_key: c.case_key,
                })
                .collect();
            return Json(CasesResponse { items, total }).into_response();
        }
        other => {
            let execution_state = other.and_then(ExecutionState::from_str_lowercase);
            let filter = CaseFilter {
                execution_state,
                limit: Some(limit),
                offset: Some(offset),
                ..Default::default()
            };
            state.case_store.list(filter).await.unwrap_or_default()
        }
    };

    // For non-"active" states we need total count (without limit/offset)
    let total_filter = CaseFilter {
        execution_state: params
            .state
            .as_deref()
            .and_then(ExecutionState::from_str_lowercase),
        ..Default::default()
    };
    let total = state
        .case_store
        .count(total_filter)
        .await
        .unwrap_or(cases.len() as u64) as usize;

    let items: Vec<CaseRow> = cases
        .into_iter()
        .map(|c| CaseRow {
            age_secs: (now - c.created_at).num_seconds(),
            workflow_code: c.workflow_code.clone(),
            state: c.execution_state.to_string(),
            created_at: c.created_at,
            session_id: c.session_id.clone(),
            parent_key: c.parent_key.clone(),
            child_count: c.child_keys.len(),
            resource_data: c.resource_data.clone(),
            case_key: c.case_key,
        })
        .collect();

    Json(CasesResponse { items, total }).into_response()
}

// ── GET /api/stats ────────────────────────────────────────────────────────────

/// Query parameters for `/api/stats`.
#[derive(Debug, Deserialize)]
pub struct StatsParams {
    /// Time window: "1d", "3d", or "7d" (default "1d").
    pub window: Option<String>,
}

fn parse_window_days(s: Option<&str>) -> u32 {
    match s {
        Some("3d") => 3,
        Some("7d") => 7,
        _ => 1,
    }
}

/// Return computed stats for the given time window.
pub async fn get_stats(
    State(state): State<AppState>,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let window_days = parse_window_days(params.window.as_deref());
    let stats = compute_stats(state.case_store.as_ref(), &state.observer, window_days).await;
    Json(stats).into_response()
}
