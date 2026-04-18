// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value as JsonValue;

/// A single case row returned by the `/api/cases` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CaseRow {
    pub case_key: String,
    pub workflow_code: String,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub age_secs: i64,
    pub session_id: String,
    pub parent_key: Option<String>,
    pub child_count: usize,
    pub resource_data: Option<JsonValue>,
}

/// Response body for the `/api/stats` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct StatsResponse {
    pub running_count: u64,
    pub waiting_count: u64,
    pub finished_count: u64,
    pub completion_rate: f64,
    pub avg_duration_ms: f64,
    pub p50_duration_ms: f64,
    pub p99_duration_ms: f64,
    pub avg_probe_ms: f64,
    pub window_days: u32,
}
