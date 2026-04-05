use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// A first-class session representing a group of related workflow cases.
///
/// Sessions provide identity and metadata for a conversation/interaction context.
/// All cases within a session share the same `session_id`.
///
/// The `metadata` field uses flexible JSON so users can attach business-specific
/// data (user info, channel, tags, etc.) without engine schema changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    /// Business-specific metadata. Organization is user-controlled.
    pub metadata: Option<JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn new(session_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            session_id: session_id.into(),
            metadata: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_metadata(mut self, metadata: JsonValue) -> Self {
        self.metadata = Some(metadata);
        self
    }
}
