// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use deadpool_postgres::Pool;
use tokio_postgres::Row;
use tracing::{debug, info};

use tianshu::agent::{Agent, AgentId, Capabilities};
use tianshu::store::AgentStore;

pub struct PostgresAgentStore {
    pool: Pool,
}

impl PostgresAgentStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Run the required migrations. Call once during setup.
    pub async fn setup(&self) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                include_str!("../../../migrations/005_create_wf_agents.sql"),
                &[],
            )
            .await?;
        info!("wf_agents table ready");
        Ok(())
    }

    fn row_to_agent(row: &Row) -> Result<Agent> {
        let capabilities: Capabilities =
            serde_json::from_value(row.get::<_, serde_json::Value>("capabilities"))?;

        let child_agent_ids: Vec<AgentId> = row
            .get::<_, Option<serde_json::Value>>("child_agent_ids")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let config: Option<serde_json::Value> = row.get("config");

        let parent_agent_id: Option<AgentId> =
            row.get::<_, Option<String>>("parent_agent_id").map(AgentId);

        Ok(Agent {
            agent_id: AgentId(row.get("agent_id")),
            role: row.get("role"),
            capabilities,
            parent_agent_id,
            child_agent_ids,
            config,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
    }
}

#[async_trait]
impl AgentStore for PostgresAgentStore {
    async fn upsert(&self, agent: &Agent) -> Result<()> {
        let client = self.pool.get().await?;
        debug!(
            "Upserting agent: agent_id={}, role={}",
            agent.agent_id.as_str(),
            agent.role
        );

        let parent_id: Option<String> = agent.parent_agent_id.as_ref().map(|id| id.0.clone());
        let capabilities_val = serde_json::to_value(&agent.capabilities)?;
        let child_ids_val = serde_json::to_value(&agent.child_agent_ids)?;

        client
            .execute(
                r#"
                INSERT INTO wf_agents (
                    agent_id, role, capabilities,
                    parent_agent_id, child_agent_ids, config,
                    created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (agent_id) DO UPDATE SET
                    role            = EXCLUDED.role,
                    capabilities    = EXCLUDED.capabilities,
                    parent_agent_id = EXCLUDED.parent_agent_id,
                    child_agent_ids = EXCLUDED.child_agent_ids,
                    config          = EXCLUDED.config,
                    updated_at      = EXCLUDED.updated_at
                "#,
                &[
                    &agent.agent_id.as_str(),
                    &agent.role,
                    &capabilities_val,
                    &parent_id,
                    &child_ids_val,
                    &agent.config,
                    &agent.created_at,
                    &agent.updated_at,
                ],
            )
            .await?;

        info!("Upserted agent: agent_id={}", agent.agent_id.as_str());
        Ok(())
    }

    async fn get(&self, agent_id: &AgentId) -> Result<Option<Agent>> {
        let client = self.pool.get().await?;
        debug!("Fetching agent by id: {}", agent_id.as_str());

        let row_opt = client
            .query_opt(
                "SELECT * FROM wf_agents WHERE agent_id = $1",
                &[&agent_id.as_str()],
            )
            .await?;

        Ok(row_opt.map(|r| Self::row_to_agent(&r)).transpose()?)
    }

    async fn get_by_session(&self, session_id: &str) -> Result<Vec<Agent>> {
        let client = self.pool.get().await?;
        debug!("Fetching agents for session: {}", session_id);

        let rows = client
            .query(
                r#"
                SELECT a.*
                FROM wf_agents a
                JOIN wf_cases c ON c.case_key = a.agent_id
                WHERE c.session_id = $1
                ORDER BY a.created_at ASC
                "#,
                &[&session_id],
            )
            .await?;

        rows.iter()
            .map(Self::row_to_agent)
            .collect::<Result<Vec<_>>>()
    }

    async fn delete(&self, agent_id: &AgentId) -> Result<()> {
        let client = self.pool.get().await?;
        let count = client
            .execute(
                "DELETE FROM wf_agents WHERE agent_id = $1",
                &[&agent_id.as_str()],
            )
            .await?;
        info!(
            "Deleted {} agent(s) for agent_id={}",
            count,
            agent_id.as_str()
        );
        Ok(())
    }
}
