CREATE TABLE IF NOT EXISTS wf_agent_messages (
    message_id TEXT PRIMARY KEY,
    from_agent_id TEXT NOT NULL REFERENCES wf_agents(agent_id) ON DELETE CASCADE,
    to_agent_id TEXT NOT NULL REFERENCES wf_agents(agent_id) ON DELETE CASCADE,
    payload JSONB NOT NULL,
    consumed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS wf_agent_messages_inbox ON wf_agent_messages(to_agent_id, consumed, created_at);
