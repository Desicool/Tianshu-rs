// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use chrono::Utc;
use tianshu::case::ExecutionState;
use tianshu::store::{CaseFilter, CaseStore};
use tianshu_observe::InMemoryObserver;

use crate::types::StatsResponse;

/// Compute dashboard statistics over the given time window.
///
/// `window_days` must be 1, 3, or 7. All timestamps are compared against `Utc::now()`.
pub async fn compute_stats(
    case_store: &dyn CaseStore,
    observer: &InMemoryObserver,
    window_days: u32,
) -> StatsResponse {
    let now = Utc::now();
    let window_start = now - chrono::Duration::days(window_days as i64);

    // ── Counts ────────────────────────────────────────────────────────────────

    let running_count = case_store
        .count(CaseFilter {
            execution_state: Some(ExecutionState::Running),
            ..Default::default()
        })
        .await
        .unwrap_or(0);

    let waiting_count = case_store
        .count(CaseFilter {
            execution_state: Some(ExecutionState::Waiting),
            ..Default::default()
        })
        .await
        .unwrap_or(0);

    // Finished cases updated within the window
    let finished_cases = case_store
        .list(CaseFilter {
            execution_state: Some(ExecutionState::Finished),
            updated_after: Some(window_start),
            ..Default::default()
        })
        .await
        .unwrap_or_default();
    let finished_count = finished_cases.len() as u64;

    // Active (Running+Waiting) cases created within the window
    let active_running = case_store
        .list(CaseFilter {
            execution_state: Some(ExecutionState::Running),
            created_after: Some(window_start),
            ..Default::default()
        })
        .await
        .unwrap_or_default();
    let active_waiting = case_store
        .list(CaseFilter {
            execution_state: Some(ExecutionState::Waiting),
            created_after: Some(window_start),
            ..Default::default()
        })
        .await
        .unwrap_or_default();
    let active_count = (active_running.len() + active_waiting.len()) as u64;

    // ── Completion rate ───────────────────────────────────────────────────────

    let completion_rate = if finished_count == 0 && active_count == 0 {
        0.0
    } else {
        finished_count as f64 / (finished_count + active_count) as f64
    };

    // ── Duration percentiles ──────────────────────────────────────────────────

    let mut durations_ms: Vec<u64> = finished_cases
        .iter()
        .map(|c| {
            let diff = c.updated_at - c.created_at;
            diff.num_milliseconds().max(0) as u64
        })
        .collect();
    durations_ms.sort_unstable();

    let n = durations_ms.len();
    let avg_duration_ms = if n == 0 {
        0.0
    } else {
        durations_ms.iter().sum::<u64>() as f64 / n as f64
    };
    let p50_duration_ms = if n == 0 {
        0.0
    } else {
        durations_ms[n / 2] as f64
    };
    let p99_duration_ms = if n == 0 {
        0.0
    } else {
        durations_ms[(n * 99) / 100] as f64
    };

    // ── Probe latency ─────────────────────────────────────────────────────────

    let probe_records = observer.probe_records();
    let window_probes: Vec<_> = probe_records
        .iter()
        .filter(|r| r.timestamp > window_start)
        .collect();
    let avg_probe_ms = if window_probes.is_empty() {
        0.0
    } else {
        window_probes.iter().map(|r| r.duration_ms).sum::<u64>() as f64 / window_probes.len() as f64
    };

    StatsResponse {
        running_count,
        waiting_count,
        finished_count,
        completion_rate,
        avg_duration_ms,
        p50_duration_ms,
        p99_duration_ms,
        avg_probe_ms,
        window_days,
    }
}
