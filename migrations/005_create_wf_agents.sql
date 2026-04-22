CREATE TABLE IF NOT EXISTS wf_agents (
    agent_id TEXT PRIMARY KEY REFERENCES wf_cases(case_key) ON DELETE CASCADE,
    role TEXT NOT NULL,
    capabilities JSONB NOT NULL,
    parent_agent_id TEXT REFERENCES wf_agents(agent_id),
    child_agent_ids JSONB NOT NULL DEFAULT '[]',
    config JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS wf_agents_parent_idx ON wf_agents(parent_agent_id);
CREATE INDEX IF NOT EXISTS wf_agents_session_idx ON wf_agents(agent_id);
