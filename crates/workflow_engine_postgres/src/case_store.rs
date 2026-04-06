use anyhow::Result;
use async_trait::async_trait;
use deadpool_postgres::Pool;
use tokio_postgres::Row;
use tracing::{debug, info};

use workflow_engine::case::{Case, ExecutionState};
use workflow_engine::store::CaseStore;

pub struct PostgresCaseStore {
    pool: Pool,
}

impl PostgresCaseStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    fn row_to_case(row: &Row) -> Result<Case> {
        let state_raw: String = row.get("execution_state");
        let execution_state =
            ExecutionState::from_str_lowercase(&state_raw).unwrap_or(ExecutionState::Running);

        Ok(Case {
            case_key: row.get("case_key"),
            session_id: row.get("session_id"),
            workflow_code: row.get("workflow_code"),
            execution_state,
            finished_type: row.get("finished_type"),
            finished_description: row.get("finished_description"),
            parent_key: row.get("parent_key"),
            child_keys: row
                .get::<_, Option<serde_json::Value>>("child_keys")
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default(),
            lifecycle_state: row.get("lifecycle_state"),
            processing_report: row
                .get::<_, Option<serde_json::Value>>("processing_report")
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default(),
            resource_data: row.get("resource_data"),
            private_vars: row.get("private_vars"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            depth: row.get::<_, i32>("depth") as u32,
        })
    }
}

#[async_trait]
impl CaseStore for PostgresCaseStore {
    async fn upsert(&self, case: &Case) -> Result<()> {
        let client = self.pool.get().await?;
        debug!(
            "Upserting case: case_key={}, state={:?}",
            case.case_key, case.execution_state
        );

        client
            .execute(
                r#"
                INSERT INTO wf_cases (
                    case_key, session_id, workflow_code,
                    execution_state, finished_type, finished_description,
                    parent_key, child_keys, lifecycle_state,
                    processing_report, resource_data, private_vars,
                    created_at, updated_at, depth
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                ON CONFLICT (case_key) DO UPDATE SET
                    session_id         = EXCLUDED.session_id,
                    workflow_code      = EXCLUDED.workflow_code,
                    execution_state    = EXCLUDED.execution_state,
                    finished_type      = EXCLUDED.finished_type,
                    finished_description = EXCLUDED.finished_description,
                    parent_key         = EXCLUDED.parent_key,
                    child_keys         = EXCLUDED.child_keys,
                    lifecycle_state    = EXCLUDED.lifecycle_state,
                    processing_report  = EXCLUDED.processing_report,
                    resource_data      = EXCLUDED.resource_data,
                    private_vars       = EXCLUDED.private_vars,
                    updated_at         = EXCLUDED.updated_at,
                    depth              = EXCLUDED.depth
                "#,
                &[
                    &case.case_key,
                    &case.session_id,
                    &case.workflow_code,
                    &case.execution_state.to_string(),
                    &case.finished_type,
                    &case.finished_description,
                    &case.parent_key,
                    &serde_json::to_value(&case.child_keys)?,
                    &case.lifecycle_state,
                    &serde_json::to_value(&case.processing_report)?,
                    &case.resource_data,
                    &case.private_vars,
                    &case.created_at,
                    &case.updated_at,
                    &(case.depth as i32),
                ],
            )
            .await?;

        info!("Upserted case: case_key={}", case.case_key);
        Ok(())
    }

    async fn get_by_key(&self, case_key: &str) -> Result<Option<Case>> {
        let client = self.pool.get().await?;
        debug!("Fetching case by key: {}", case_key);

        let row_opt = client
            .query_opt("SELECT * FROM wf_cases WHERE case_key = $1", &[&case_key])
            .await?;

        Ok(row_opt.map(|r| Self::row_to_case(&r)).transpose()?)
    }

    async fn get_by_session(&self, session_id: &str) -> Result<Vec<Case>> {
        let client = self.pool.get().await?;
        debug!("Fetching cases for session: {}", session_id);

        let rows = client
            .query(
                "SELECT * FROM wf_cases WHERE session_id = $1 ORDER BY created_at ASC",
                &[&session_id],
            )
            .await?;

        rows.iter()
            .map(Self::row_to_case)
            .collect::<Result<Vec<_>>>()
    }

    async fn setup(&self) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS wf_cases (
                    case_key            TEXT PRIMARY KEY,
                    session_id          TEXT NOT NULL,
                    workflow_code       TEXT NOT NULL,
                    execution_state     TEXT NOT NULL DEFAULT 'running',
                    finished_type       TEXT,
                    finished_description TEXT,
                    parent_key          TEXT,
                    child_keys          JSONB NOT NULL DEFAULT '[]'::jsonb,
                    lifecycle_state     TEXT NOT NULL DEFAULT 'normal',
                    processing_report   JSONB NOT NULL DEFAULT '[]'::jsonb,
                    resource_data       JSONB,
                    private_vars        JSONB,
                    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    depth               INTEGER NOT NULL DEFAULT 0
                )
                -- Migration for existing tables:
                -- ALTER TABLE wf_cases ADD COLUMN depth INTEGER NOT NULL DEFAULT 0;
                "#,
                &[],
            )
            .await?;
        client
            .execute(
                "CREATE INDEX IF NOT EXISTS wf_cases_session_id_idx ON wf_cases (session_id)",
                &[],
            )
            .await?;
        info!("wf_cases table ready");
        Ok(())
    }
}
