-- Migration 003: workflow sessions table
--
-- Sessions group related workflow cases and carry metadata.
-- The schema here is a reference implementation; users with complex
-- session structures should adapt to their business requirements.

CREATE TABLE IF NOT EXISTS wf_sessions (
    session_id  TEXT PRIMARY KEY,
    metadata    JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
