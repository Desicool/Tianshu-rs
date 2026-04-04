-- Migration 002: workflow runtime state table
--
-- Stores per-case, per-step serialised data (checkpoints, state variables).
-- Run this manually or use PostgresStateStore::setup() for automatic creation.

CREATE TABLE IF NOT EXISTS wf_state (
    case_key   TEXT NOT NULL,
    step       TEXT NOT NULL,
    data       TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (case_key, step)
);

CREATE INDEX IF NOT EXISTS wf_state_case_key_idx ON wf_state (case_key);
