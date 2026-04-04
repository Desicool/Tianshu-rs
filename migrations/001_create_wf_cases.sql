-- Migration 001: workflow case table
--
-- Run this manually if you prefer to manage schema yourself,
-- or use PostgresCaseStore::setup() for automatic creation.

CREATE TABLE IF NOT EXISTS wf_cases (
    case_key             TEXT PRIMARY KEY,
    session_id           TEXT NOT NULL,
    workflow_code        TEXT NOT NULL,
    execution_state      TEXT NOT NULL DEFAULT 'running',
    finished_type        TEXT,
    finished_description TEXT,
    parent_key           TEXT,
    child_key            TEXT,
    lifecycle_state      TEXT NOT NULL DEFAULT 'normal',
    processing_report    JSONB NOT NULL DEFAULT '[]'::jsonb,
    resource_data        JSONB,
    private_vars         JSONB,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS wf_cases_session_id_idx ON wf_cases (session_id);
