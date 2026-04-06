// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use deadpool_postgres::Pool;
use tracing::{debug, info};

use tianshu::session::Session;
use tianshu::store::SessionStore;

/// Reference PostgreSQL implementation of `SessionStore`.
///
/// Uses a simple `wf_sessions` table with `metadata JSONB`. Production users
/// with complex session schemas should implement their own `SessionStore`.
pub struct PostgresSessionStore {
    pool: Pool,
}

impl PostgresSessionStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStore for PostgresSessionStore {
    async fn upsert(&self, session: &Session) -> Result<()> {
        let client = self.pool.get().await?;
        debug!("Upserting session: session_id={}", session.session_id);

        client
            .execute(
                r#"
                INSERT INTO wf_sessions (session_id, metadata, created_at, updated_at)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (session_id) DO UPDATE SET
                    metadata   = EXCLUDED.metadata,
                    updated_at = EXCLUDED.updated_at
                "#,
                &[
                    &session.session_id,
                    &session.metadata,
                    &session.created_at,
                    &session.updated_at,
                ],
            )
            .await?;

        info!("Upserted session: session_id={}", session.session_id);
        Ok(())
    }

    async fn get(&self, session_id: &str) -> Result<Option<Session>> {
        let client = self.pool.get().await?;
        debug!("Fetching session: session_id={}", session_id);

        let row_opt = client
            .query_opt(
                "SELECT session_id, metadata, created_at, updated_at FROM wf_sessions WHERE session_id = $1",
                &[&session_id],
            )
            .await?;

        Ok(row_opt.map(|row| Session {
            session_id: row.get(0),
            metadata: row.get(1),
            created_at: row.get(2),
            updated_at: row.get(3),
        }))
    }

    async fn delete(&self, session_id: &str) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "DELETE FROM wf_sessions WHERE session_id = $1",
                &[&session_id],
            )
            .await?;
        info!("Deleted session: session_id={}", session_id);
        Ok(())
    }

    async fn setup(&self) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS wf_sessions (
                    session_id  TEXT PRIMARY KEY,
                    metadata    JSONB,
                    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )
                "#,
                &[],
            )
            .await?;
        info!("wf_sessions table ready");
        Ok(())
    }
}
