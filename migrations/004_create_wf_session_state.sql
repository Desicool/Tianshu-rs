-- Migration 004: session-scoped workflow runtime state
--
-- Stores per-session, per-step serialised data for cross-case variables.
-- No engine-level locking is provided; workflows manage their own concurrency.

CREATE TABLE IF NOT EXISTS wf_session_state (
    session_id TEXT NOT NULL,
    step       TEXT NOT NULL,
    data       TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, step)
);

CREATE INDEX IF NOT EXISTS wf_session_state_session_id_idx ON wf_session_state (session_id);
