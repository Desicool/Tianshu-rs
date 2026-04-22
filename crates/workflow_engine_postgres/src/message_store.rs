// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use deadpool_postgres::Pool;
use tokio_postgres::Row;
use tracing::{debug, info};

use tianshu::agent::AgentId;
use tianshu::store::{AgentMessage, AgentMessageStore};

pub struct PostgresAgentMessageStore {
    pool: Pool,
}

impl PostgresAgentMessageStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn setup(&self) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                include_str!("../../../migrations/006_create_wf_agent_messages.sql"),
                &[],
            )
            .await?;
        info!("wf_agent_messages table ready");
        Ok(())
    }

    fn row_to_message(row: &Row) -> Result<AgentMessage> {
        Ok(AgentMessage {
            message_id: row.get("message_id"),
            from_agent_id: AgentId(row.get("from_agent_id")),
            to_agent_id: AgentId(row.get("to_agent_id")),
            payload: row.get("payload"),
            consumed: row.get("consumed"),
            created_at: row.get("created_at"),
        })
    }
}

#[async_trait]
impl AgentMessageStore for PostgresAgentMessageStore {
    async fn send(&self, message: &AgentMessage) -> Result<()> {
        let client = self.pool.get().await?;
        debug!(
            "Sending message: message_id={}, from={}, to={}",
            message.message_id,
            message.from_agent_id.as_str(),
            message.to_agent_id.as_str()
        );

        client
            .execute(
                r#"
                INSERT INTO wf_agent_messages (
                    message_id, from_agent_id, to_agent_id,
                    payload, consumed, created_at
                )
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (message_id) DO NOTHING
                "#,
                &[
                    &message.message_id,
                    &message.from_agent_id.as_str(),
                    &message.to_agent_id.as_str(),
                    &message.payload,
                    &message.consumed,
                    &message.created_at,
                ],
            )
            .await?;

        info!("Sent message: message_id={}", message.message_id);
        Ok(())
    }

    async fn receive(&self, to: &AgentId) -> Result<Vec<AgentMessage>> {
        let client = self.pool.get().await?;
        debug!("Receiving unconsumed messages for agent: {}", to.as_str());

        let rows = client
            .query(
                r#"
                SELECT message_id, from_agent_id, to_agent_id,
                       payload, consumed, created_at
                FROM wf_agent_messages
                WHERE to_agent_id = $1 AND consumed = false
                ORDER BY created_at ASC
                "#,
                &[&to.as_str()],
            )
            .await?;

        rows.iter()
            .map(Self::row_to_message)
            .collect::<Result<Vec<_>>>()
    }

    async fn acknowledge(&self, message_ids: &[String]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let client = self.pool.get().await?;
        debug!("Acknowledging {} message(s)", message_ids.len());

        // tokio-postgres ANY($1) requires a slice reference of the right type.
        let ids: Vec<&str> = message_ids.iter().map(String::as_str).collect();
        let count = client
            .execute(
                "UPDATE wf_agent_messages SET consumed = true WHERE message_id = ANY($1)",
                &[&ids],
            )
            .await?;

        info!("Acknowledged {} message(s)", count);
        Ok(())
    }

    async fn delete_by_agent(&self, agent_id: &AgentId) -> Result<()> {
        let client = self.pool.get().await?;
        let count = client
            .execute(
                "DELETE FROM wf_agent_messages WHERE from_agent_id = $1 OR to_agent_id = $1",
                &[&agent_id.as_str()],
            )
            .await?;
        info!(
            "Deleted {} message(s) for agent_id={}",
            count,
            agent_id.as_str()
        );
        Ok(())
    }
}
