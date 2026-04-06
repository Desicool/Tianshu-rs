// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::RwLock;

use crate::case::Case;
use crate::session::Session;

// ── CaseStore ────────────────────────────────────────────────────────────────

/// Manages workflow case lifecycle.
///
/// Cases are structured records with typed fields, queried by key or session.
/// Implementations can target any database (PostgreSQL, MySQL, SQLite, MongoDB …).
#[async_trait]
pub trait CaseStore: Send + Sync {
    /// Insert or update a case record.
    async fn upsert(&self, case: &Case) -> Result<()>;

    /// Fetch a single case by its unique key.
    async fn get_by_key(&self, case_key: &str) -> Result<Option<Case>>;

    /// List all cases belonging to a session.
    async fn get_by_session(&self, session_id: &str) -> Result<Vec<Case>>;

    /// Optional: create tables / collections / indexes on first use.
    async fn setup(&self) -> Result<()> {
        Ok(())
    }
}

// ── SessionStore ─────────────────────────────────────────────────────────────

/// Manages session lifecycle.
///
/// Sessions group related workflow cases. This is a minimal abstraction —
/// the engine only needs basic CRUD. Users should implement this trait to
/// match their business-specific schema, since session structure is highly
/// coupled to business logic.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Insert or update a session record.
    async fn upsert(&self, session: &Session) -> Result<()>;

    /// Fetch a session by its unique ID.
    async fn get(&self, session_id: &str) -> Result<Option<Session>>;

    /// Delete a session by its unique ID.
    async fn delete(&self, session_id: &str) -> Result<()>;

    /// Optional: create tables / collections / indexes on first use.
    async fn setup(&self) -> Result<()> {
        Ok(())
    }
}

// ── StateStore ───────────────────────────────────────────────────────────────

/// A single workflow state entry: one step's persisted data for one case.
#[derive(Debug, Clone)]
pub struct StateEntry {
    pub case_key: String,
    pub step: String,
    /// JSON-serialized (or otherwise encoded) workflow data.
    pub data: String,
    pub updated_at: DateTime<Utc>,
}

/// A session-scoped state entry: shared data across all cases in a session.
#[derive(Debug, Clone)]
pub struct SessionStateEntry {
    pub session_id: String,
    pub step: String,
    /// JSON-serialized workflow data.
    pub data: String,
    pub updated_at: DateTime<Utc>,
}

/// Stores workflow runtime data — step outputs, variables, intermediate results.
///
/// Data is keyed by `(case_key, step)` and stored as serialized strings.
/// This is NOT just checkpoints: workflows actively read and write this data
/// throughout their entire execution lifecycle.
///
/// Implementations can target any store (PostgreSQL JSONB, Redis hash, DynamoDB …).
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Save or update state for a specific case + step pair.
    async fn save(&self, case_key: &str, step: &str, data: &str) -> Result<()>;

    /// Fetch state for a specific case + step. Returns `None` if not found.
    async fn get(&self, case_key: &str, step: &str) -> Result<Option<StateEntry>>;

    /// Fetch all state entries for a case (every step).
    async fn get_all(&self, case_key: &str) -> Result<Vec<StateEntry>>;

    /// Delete all state entries for a case (cleanup when workflow finishes).
    async fn delete_by_case(&self, case_key: &str) -> Result<()>;

    // ── Session-scoped state (cross-case variables) ────────────────────────

    /// Save session-scoped state for a specific session + step pair.
    ///
    /// No engine-level locking is provided. Workflows are responsible for
    /// their own concurrency control when using session-scoped variables.
    async fn save_session(&self, _session_id: &str, _step: &str, _data: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "session-scoped state not supported by this store"
        ))
    }

    /// Fetch session-scoped state for a specific session + step.
    async fn get_session(
        &self,
        _session_id: &str,
        _step: &str,
    ) -> Result<Option<SessionStateEntry>> {
        Err(anyhow::anyhow!(
            "session-scoped state not supported by this store"
        ))
    }

    /// Fetch all session-scoped state entries for a session.
    async fn get_all_session(&self, _session_id: &str) -> Result<Vec<SessionStateEntry>> {
        Err(anyhow::anyhow!(
            "session-scoped state not supported by this store"
        ))
    }

    /// Delete all session-scoped state entries for a session.
    async fn delete_by_session(&self, _session_id: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "session-scoped state not supported by this store"
        ))
    }

    /// Optional: create tables / collections / indexes on first use.
    async fn setup(&self) -> Result<()> {
        Ok(())
    }
}

// ── InMemoryCaseStore ────────────────────────────────────────────────────────

/// In-memory CaseStore for development and testing.
///
/// Not suitable for production (data is lost on restart).
#[derive(Default)]
pub struct InMemoryCaseStore {
    // case_key → Case
    cases: RwLock<HashMap<String, Case>>,
}

#[async_trait]
impl CaseStore for InMemoryCaseStore {
    async fn upsert(&self, case: &Case) -> Result<()> {
        let mut guard = self.cases.write().unwrap();
        guard.insert(case.case_key.clone(), case.clone());
        Ok(())
    }

    async fn get_by_key(&self, case_key: &str) -> Result<Option<Case>> {
        let guard = self.cases.read().unwrap();
        Ok(guard.get(case_key).cloned())
    }

    async fn get_by_session(&self, session_id: &str) -> Result<Vec<Case>> {
        let guard = self.cases.read().unwrap();
        let result = guard
            .values()
            .filter(|c| c.session_id == session_id)
            .cloned()
            .collect();
        Ok(result)
    }
}

// ── InMemorySessionStore ─────────────────────────────────────────────────────

/// In-memory SessionStore for development and testing.
///
/// This is a reference implementation. Production users should implement
/// `SessionStore` with their business-specific schema.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<String, Session>>,
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn upsert(&self, session: &Session) -> Result<()> {
        let mut guard = self.sessions.write().unwrap();
        guard.insert(session.session_id.clone(), session.clone());
        Ok(())
    }

    async fn get(&self, session_id: &str) -> Result<Option<Session>> {
        let guard = self.sessions.read().unwrap();
        Ok(guard.get(session_id).cloned())
    }

    async fn delete(&self, session_id: &str) -> Result<()> {
        let mut guard = self.sessions.write().unwrap();
        guard.remove(session_id);
        Ok(())
    }
}

// ── InMemoryStateStore ───────────────────────────────────────────────────────

/// In-memory StateStore for development and testing.
///
/// Not suitable for production (data is lost on restart).
#[derive(Default)]
pub struct InMemoryStateStore {
    // (case_key, step) → StateEntry
    entries: RwLock<HashMap<(String, String), StateEntry>>,
    // (session_id, step) → SessionStateEntry
    session_entries: RwLock<HashMap<(String, String), SessionStateEntry>>,
}

#[async_trait]
impl StateStore for InMemoryStateStore {
    async fn save(&self, case_key: &str, step: &str, data: &str) -> Result<()> {
        let mut guard = self.entries.write().unwrap();
        guard.insert(
            (case_key.to_string(), step.to_string()),
            StateEntry {
                case_key: case_key.to_string(),
                step: step.to_string(),
                data: data.to_string(),
                updated_at: Utc::now(),
            },
        );
        Ok(())
    }

    async fn get(&self, case_key: &str, step: &str) -> Result<Option<StateEntry>> {
        let guard = self.entries.read().unwrap();
        Ok(guard
            .get(&(case_key.to_string(), step.to_string()))
            .cloned())
    }

    async fn get_all(&self, case_key: &str) -> Result<Vec<StateEntry>> {
        let guard = self.entries.read().unwrap();
        let result = guard
            .values()
            .filter(|e| e.case_key == case_key)
            .cloned()
            .collect();
        Ok(result)
    }

    async fn delete_by_case(&self, case_key: &str) -> Result<()> {
        let mut guard = self.entries.write().unwrap();
        guard.retain(|(ck, _), _| ck != case_key);
        Ok(())
    }

    async fn save_session(&self, session_id: &str, step: &str, data: &str) -> Result<()> {
        let mut guard = self.session_entries.write().unwrap();
        guard.insert(
            (session_id.to_string(), step.to_string()),
            SessionStateEntry {
                session_id: session_id.to_string(),
                step: step.to_string(),
                data: data.to_string(),
                updated_at: Utc::now(),
            },
        );
        Ok(())
    }

    async fn get_session(&self, session_id: &str, step: &str) -> Result<Option<SessionStateEntry>> {
        let guard = self.session_entries.read().unwrap();
        Ok(guard
            .get(&(session_id.to_string(), step.to_string()))
            .cloned())
    }

    async fn get_all_session(&self, session_id: &str) -> Result<Vec<SessionStateEntry>> {
        let guard = self.session_entries.read().unwrap();
        let result = guard
            .values()
            .filter(|e| e.session_id == session_id)
            .cloned()
            .collect();
        Ok(result)
    }

    async fn delete_by_session(&self, session_id: &str) -> Result<()> {
        let mut guard = self.session_entries.write().unwrap();
        guard.retain(|(sid, _), _| sid != session_id);
        Ok(())
    }
}