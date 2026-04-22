// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fmt;
use std::hash::Hash;

use crate::case::Case;

/// Unique identifier for an agent. Wraps the backing Case's case_key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Capability restrictions that can be applied when spawning a child agent.
/// Fields use Option so callers only specify what they want to restrict.
/// A child's effective capabilities = parent.narrow(restriction) — never expands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRestriction {
    /// If Some, child's allowed_tools is this list intersected with parent's.
    pub allowed_tools: Option<Vec<String>>,
    /// If Some(false), child cannot spawn. Cannot be set to true if parent is false.
    pub can_spawn: Option<bool>,
    /// If Some, child's max_spawn_depth is min(parent.max_spawn_depth - 1, this).
    pub max_spawn_depth: Option<u32>,
    /// If Some(false), child cannot send messages. Cannot expand parent.
    pub can_send_messages: Option<bool>,
    /// If Some, child's visible_session_keys is this list intersected with parent's.
    pub visible_session_keys: Option<Vec<String>>,
    /// If Some, child's writable_session_keys is this list intersected with parent's.
    pub writable_session_keys: Option<Vec<String>>,
}

/// Capability set for an agent. Controls what the agent is allowed to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    /// Tool names this agent can access. ["*"] means all tools.
    pub allowed_tools: Vec<String>,
    /// Whether this agent can spawn child agents.
    pub can_spawn: bool,
    /// Maximum depth of agent spawning (0 = cannot spawn).
    pub max_spawn_depth: u32,
    /// Whether this agent can send messages to other agents.
    pub can_send_messages: bool,
    /// Session state keys visible to this agent. ["*"] means all.
    pub visible_session_keys: Vec<String>,
    /// Session state keys this agent can write. Must be subset of visible_session_keys.
    pub writable_session_keys: Vec<String>,
    /// User-defined policy extension data.
    pub extensions: Option<JsonValue>,
}

impl Capabilities {
    /// Root capabilities: unrestricted access to everything.
    pub fn root() -> Self {
        Self {
            allowed_tools: vec!["*".to_string()],
            can_spawn: true,
            max_spawn_depth: 8,
            can_send_messages: true,
            visible_session_keys: vec!["*".to_string()],
            writable_session_keys: vec!["*".to_string()],
            extensions: None,
        }
    }

    /// Narrow capabilities by applying a restriction. Can only restrict, never expand.
    pub fn narrow(&self, restriction: &CapabilityRestriction) -> Self {
        let allowed_tools = if let Some(restricted) = &restriction.allowed_tools {
            if self.allowed_tools == vec!["*".to_string()] {
                restricted.clone()
            } else {
                // intersection: only keep what parent allows
                self.allowed_tools
                    .iter()
                    .filter(|t| restricted.contains(t))
                    .cloned()
                    .collect()
            }
        } else {
            self.allowed_tools.clone()
        };

        let can_spawn = match restriction.can_spawn {
            Some(false) => false,
            _ => self.can_spawn,
        };

        // max_spawn_depth always decrements by 1 when crossing an agent boundary,
        // then applies the restriction as an additional ceiling.
        let parent_depth = self.max_spawn_depth.saturating_sub(1);
        let max_spawn_depth = if let Some(d) = restriction.max_spawn_depth {
            parent_depth.min(d)
        } else {
            parent_depth
        };

        let can_send_messages = match restriction.can_send_messages {
            Some(false) => false,
            _ => self.can_send_messages,
        };

        let visible_session_keys = if let Some(restricted) = &restriction.visible_session_keys {
            if self.visible_session_keys == vec!["*".to_string()] {
                restricted.clone()
            } else {
                self.visible_session_keys
                    .iter()
                    .filter(|k| restricted.contains(k))
                    .cloned()
                    .collect()
            }
        } else {
            self.visible_session_keys.clone()
        };

        let writable_session_keys = if let Some(restricted) = &restriction.writable_session_keys {
            if self.writable_session_keys == vec!["*".to_string()] {
                restricted.clone()
            } else {
                self.writable_session_keys
                    .iter()
                    .filter(|k| restricted.contains(k))
                    .cloned()
                    .collect()
            }
        } else {
            self.writable_session_keys.clone()
        };

        Self {
            allowed_tools,
            can_spawn,
            max_spawn_depth,
            can_send_messages,
            visible_session_keys,
            writable_session_keys,
            extensions: self.extensions.clone(),
        }
    }

    /// Whether a tool with this name is allowed.
    pub fn is_tool_allowed(&self, name: &str) -> bool {
        self.allowed_tools.iter().any(|t| t == "*" || t == name)
    }

    /// Whether a session key is visible to this agent.
    pub fn is_session_key_visible(&self, key: &str) -> bool {
        self.visible_session_keys
            .iter()
            .any(|k| k == "*" || k == key)
    }

    /// Whether a session key is writable by this agent.
    pub fn is_session_key_writable(&self, key: &str) -> bool {
        self.writable_session_keys
            .iter()
            .any(|k| k == "*" || k == key)
    }
}

/// An agent — the first-class execution unit. Backed 1:1 by a Case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique identifier = backing Case's case_key.
    pub agent_id: AgentId,
    /// Human-readable role name, e.g. "coordinator", "researcher".
    pub role: String,
    /// What this agent is allowed to do.
    pub capabilities: Capabilities,
    /// Parent agent that spawned this one, if any.
    pub parent_agent_id: Option<AgentId>,
    /// Child agents spawned by this agent.
    pub child_agent_ids: Vec<AgentId>,
    /// Agent-specific instructions or configuration.
    pub config: Option<JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Agent {
    /// Create a root agent with full capabilities, plus its backing Case.
    /// The caller must upsert both the Agent and the Case before scheduling.
    pub fn root(
        agent_id: impl Into<String>,
        role: impl Into<String>,
        session_id: impl Into<String>,
        workflow_code: impl Into<String>,
    ) -> (Agent, Case) {
        let id = agent_id.into();
        let sid = session_id.into();
        let wf = workflow_code.into();
        let now = Utc::now();

        let agent = Agent {
            agent_id: AgentId(id.clone()),
            role: role.into(),
            capabilities: Capabilities::root(),
            parent_agent_id: None,
            child_agent_ids: vec![],
            config: None,
            created_at: now,
            updated_at: now,
        };

        let case = Case::new(id, sid, wf);

        (agent, case)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_display() {
        let id = AgentId::new("agent_001");
        assert_eq!(id.to_string(), "agent_001");
        assert_eq!(id.as_str(), "agent_001");
    }

    #[test]
    fn capabilities_root_is_unrestricted() {
        let caps = Capabilities::root();
        assert!(caps.is_tool_allowed("any_tool"));
        assert!(caps.can_spawn);
        assert!(caps.can_send_messages);
        assert!(caps.is_session_key_visible("any_key"));
        assert!(caps.is_session_key_writable("any_key"));
    }

    #[test]
    fn narrow_restricts_tools() {
        let parent = Capabilities::root();
        let restriction = CapabilityRestriction {
            allowed_tools: Some(vec!["add".to_string(), "subtract".to_string()]),
            can_spawn: None,
            max_spawn_depth: None,
            can_send_messages: None,
            visible_session_keys: None,
            writable_session_keys: None,
        };
        let child = parent.narrow(&restriction);
        assert!(child.is_tool_allowed("add"));
        assert!(child.is_tool_allowed("subtract"));
        assert!(!child.is_tool_allowed("multiply"));
        assert!(!child.is_tool_allowed("*"));
    }

    #[test]
    fn narrow_cannot_expand_spawn() {
        let mut parent = Capabilities::root();
        parent.can_spawn = false;
        let restriction = CapabilityRestriction {
            allowed_tools: None,
            can_spawn: Some(true), // tries to expand — must not work
            max_spawn_depth: None,
            can_send_messages: None,
            visible_session_keys: None,
            writable_session_keys: None,
        };
        let child = parent.narrow(&restriction);
        assert!(!child.can_spawn);
    }

    #[test]
    fn narrow_decrements_spawn_depth() {
        let parent = Capabilities::root(); // max_spawn_depth = 8
        let restriction = CapabilityRestriction {
            allowed_tools: None,
            can_spawn: None,
            max_spawn_depth: None,
            can_send_messages: None,
            visible_session_keys: None,
            writable_session_keys: None,
        };
        let child = parent.narrow(&restriction);
        assert_eq!(child.max_spawn_depth, 7); // decremented by 1
    }

    #[test]
    fn agent_root_creates_matching_case() {
        let (agent, case) =
            Agent::root("agent_001", "coordinator", "session_001", "coordinator_wf");
        assert_eq!(agent.agent_id.as_str(), "agent_001");
        assert_eq!(case.case_key, "agent_001");
        assert_eq!(case.session_id, "session_001");
        assert_eq!(case.workflow_code, "coordinator_wf");
        assert!(agent.parent_agent_id.is_none());
    }
}
