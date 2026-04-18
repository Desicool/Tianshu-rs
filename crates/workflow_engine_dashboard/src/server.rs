// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use anyhow::Result;
use axum::{routing::get, Router};
use tower_http::cors::CorsLayer;
use tracing::info;

use tianshu::store::CaseStore;
use tianshu_observe::InMemoryObserver;

use crate::handlers::{get_cases, get_config, get_index, get_stats};

/// Shared state threaded through all Axum handler functions.
#[derive(Clone)]
pub struct AppState {
    pub case_store: Arc<dyn CaseStore>,
    pub observer: Arc<InMemoryObserver>,
    pub refresh_secs: u64,
}

/// A self-contained HTTP dashboard server.
///
/// Serves a dark-themed web UI at `/` and JSON APIs at `/api/*`.
///
/// # Example
///
/// ```rust,ignore
/// let server = DashboardServer::new(case_store, observer)
///     .with_port(8080)
///     .with_refresh_secs(5);
/// server.serve().await?;
/// ```
pub struct DashboardServer {
    case_store: Arc<dyn CaseStore>,
    observer: Arc<InMemoryObserver>,
    port: u16,
    refresh_secs: u64,
}

impl DashboardServer {
    /// Create a new dashboard server with default port (8765) and refresh (5s).
    pub fn new(case_store: Arc<dyn CaseStore>, observer: Arc<InMemoryObserver>) -> Self {
        Self {
            case_store,
            observer,
            port: 8765,
            refresh_secs: 5,
        }
    }

    /// Set the TCP port to listen on.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the auto-refresh interval in seconds for the web UI.
    pub fn with_refresh_secs(mut self, secs: u64) -> Self {
        self.refresh_secs = secs;
        self
    }

    /// Bind to `127.0.0.1:{port}` and start serving.
    pub async fn serve(self) -> Result<()> {
        let state = AppState {
            case_store: self.case_store,
            observer: self.observer,
            refresh_secs: self.refresh_secs,
        };

        let app = Router::new()
            .route("/", get(get_index))
            .route("/api/config", get(get_config))
            .route("/api/cases", get(get_cases))
            .route("/api/stats", get(get_stats))
            .layer(CorsLayer::permissive())
            .with_state(state);

        let addr = format!("127.0.0.1:{}", self.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        info!("tianshu-dashboard listening on http://{}", addr);

        axum::serve(listener, app).await?;
        Ok(())
    }
}
