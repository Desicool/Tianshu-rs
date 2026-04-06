// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use deadpool_postgres::Pool;
use tracing::{debug, info};

use tianshu::store::{SessionStateEntry, StateEntry, StateStore};

pub struct PostgresStateStore {
    pool: Pool,
}

impl PostgresStateStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl StateStore for PostgresStateStore {
    async fn save(&self, case_key: &str, step: &str, data: &str) -> Result<()> {
        let client = self.pool.get().await?;
        debug!("Saving state: case_key={}, step={}", case_key, step);

        let now = Utc::now();
        client
            .execute(
                r#"
                INSERT INTO wf_state (case_key, step, data, updated_at)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (case_key, step) DO UPDATE SET
                    data       = EXCLUDED.data,
                    updated_at = EXCLUDED.updated_at
                "#,
                &[&case_key, &step, &data, &now],
            )
            .await?;

        info!("Saved state: case_key={}, step={}", case_key, step);
        Ok(())
    }

    async fn get(&self, case_key: &str, step: &str) -> Result<Option<StateEntry>> {
        let client = self.pool.get().await?;
        debug!("Getting state: case_key={}, step={}", case_key, step);

        let row_opt = client
            .query_opt(
                "SELECT case_key, step, data, updated_at FROM wf_state WHERE case_key = $1 AND step = $2",
                &[&case_key, &step],
            )
            .await?;

        Ok(row_opt.map(|row| StateEntry {
            case_key: row.get(0),
            step: row.get(1),
            data: row.get(2),
            updated_at: row.get(3),
        }))
    }

    async fn get_all(&self, case_key: &str) -> Result<Vec<StateEntry>> {
        let client = self.pool.get().await?;
        debug!("Getting all state for case_key={}", case_key);

        let rows = client
            .query(
                "SELECT case_key, step, data, updated_at FROM wf_state WHERE case_key = $1",
                &[&case_key],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|row| StateEntry {
                case_key: row.get(0),
                step: row.get(1),
                data: row.get(2),
                updated_at: row.get(3),
            })
            .collect())
    }

    async fn delete_by_case(&self, case_key: &str) -> Result<()> {
        let client = self.pool.get().await?;
        let count = client
            .execute("DELETE FROM wf_state WHERE case_key = $1", &[&case_key])
            .await?;
        info!("Deleted {} state entries for case_key={}", count, case_key);
        Ok(())
    }

    async fn save_session(&self, session_id: &str, step: &str, data: &str) -> Result<()> {
        let client = self.pool.get().await?;
        debug!(
            "Saving session state: session_id={}, step={}",
            session_id, step
        );

        let now = Utc::now();
        client
            .execute(
                r#"
                INSERT INTO wf_session_state (session_id, step, data, updated_at)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (session_id, step) DO UPDATE SET
                    data       = EXCLUDED.data,
                    updated_at = EXCLUDED.updated_at
                "#,
                &[&session_id, &step, &data, &now],
            )
            .await?;

        info!(
            "Saved session state: session_id={}, step={}",
            session_id, step
        );
        Ok(())
    }

    async fn get_session(&self, session_id: &str, step: &str) -> Result<Option<SessionStateEntry>> {
        let client = self.pool.get().await?;
        let row_opt = client
            .query_opt(
                "SELECT session_id, step, data, updated_at FROM wf_session_state WHERE session_id = $1 AND step = $2",
                &[&session_id, &step],
            )
            .await?;

        Ok(row_opt.map(|row| SessionStateEntry {
            session_id: row.get(0),
            step: row.get(1),
            data: row.get(2),
            updated_at: row.get(3),
        }))
    }

    async fn get_all_session(&self, session_id: &str) -> Result<Vec<SessionStateEntry>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT session_id, step, data, updated_at FROM wf_session_state WHERE session_id = $1",
                &[&session_id],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|row| SessionStateEntry {
                session_id: row.get(0),
                step: row.get(1),
                data: row.get(2),
                updated_at: row.get(3),
            })
            .collect())
    }

    async fn delete_by_session(&self, session_id: &str) -> Result<()> {
        let client = self.pool.get().await?;
        let count = client
            .execute(
                "DELETE FROM wf_session_state WHERE session_id = $1",
                &[&session_id],
            )
            .await?;
        info!(
            "Deleted {} session state entries for session_id={}",
            count, session_id
        );
        Ok(())
    }

    async fn setup(&self) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS wf_state (
                    case_key   TEXT NOT NULL,
                    step       TEXT NOT NULL,
                    data       TEXT NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    PRIMARY KEY (case_key, step)
                )
                "#,
                &[],
            )
            .await?;
        client
            .execute(
                "CREATE INDEX IF NOT EXISTS wf_state_case_key_idx ON wf_state (case_key)",
                &[],
            )
            .await?;
        client
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS wf_session_state (
                    session_id TEXT NOT NULL,
                    step       TEXT NOT NULL,
                    data       TEXT NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    PRIMARY KEY (session_id, step)
                )
                "#,
                &[],
            )
            .await?;
        client
            .execute(
                "CREATE INDEX IF NOT EXISTS wf_session_state_session_id_idx ON wf_session_state (session_id)",
                &[],
            )
            .await?;
        info!("wf_state and wf_session_state tables ready");
        Ok(())
    }
}
