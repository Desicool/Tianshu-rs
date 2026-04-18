// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

//! HTTP dashboard for the Tianshu workflow engine.
//!
//! Provides a self-contained web UI and JSON API to inspect running cases,
//! view metrics, and monitor observer telemetry.

pub mod handlers;
pub mod metrics;
pub mod server;
pub mod types;

pub use server::DashboardServer;
pub use types::{CaseRow, StatsResponse};
