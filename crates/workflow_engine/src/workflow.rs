use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::context::WorkflowContext;

/// Result of a single workflow execution tick
#[derive(Debug, Clone)]
pub enum WorkflowResult {
    /// Workflow completed a step and can continue immediately
    Continue,
    /// Workflow is waiting for an external event (poll predicate)
    Waiting(Vec<PollPredicate>),
    /// Workflow has finished (type, description)
    Finished(String, String),
}

/// Poll predicate: defines what resource to check and how
#[derive(Debug, Clone)]
pub struct PollPredicate {
    /// Type of resource to fetch (e.g., "message", "resource_data", "case_state")
    pub resource_type: String,
    /// Identifier of the resource to poll (e.g., creator_uuid, case_key)
    pub resource_id: String,
    /// Step name this poll is associated with
    pub step_name: String,
    /// Optional description for LLM classification (intent routing)
    pub intent_desc: Option<String>,
}

/// Route listener declaration attached to a workflow
#[derive(Debug, Clone)]
pub struct RouteListener {
    /// Description used by router LLM
    pub intent_desc: String,
    /// Message filter type (currently only "creator_last_message")
    pub msg_filter: String,
    /// Whether this listener should be included in LLM route classification
    pub needs_llm: bool,
}

impl Default for RouteListener {
    fn default() -> Self {
        Self {
            intent_desc: String::new(),
            msg_filter: "creator_last_message".to_string(),
            needs_llm: true,
        }
    }
}

/// Timeout configuration for wait_for_event
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// How long to wait before each strike (default 12h = 43200s)
    pub interval_seconds: u64,
    /// Total strikes before final action (default 3)
    pub max_strikes: u32,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            interval_seconds: 43200,
            max_strikes: 3,
        }
    }
}

/// Base workflow trait. All workflow implementations must implement this.
///
/// Workflows are restart-safe through ctx.step() checkpoints.
/// run() is invoked repeatedly until it returns Finished.
#[async_trait]
pub trait BaseWorkflow: Send + Sync {
    /// Execute one tick of the workflow
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult>;

    /// Optional route listener for intent routing
    fn route_listener(&self) -> Option<&RouteListener> {
        None
    }

    /// Whether this workflow should be considered by the intent router
    fn is_listening(&self, _ctx: &WorkflowContext) -> bool {
        false
    }

    /// Called by router when this workflow is matched
    fn on_route_matched(&self, _ctx: &mut WorkflowContext, _payload: Option<JsonValue>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poll_predicate_creation() {
        let poll = PollPredicate {
            resource_type: "message".to_string(),
            resource_id: "creator_123".to_string(),
            step_name: "wait_for_reply".to_string(),
            intent_desc: Some("User wants to negotiate".to_string()),
        };
        assert_eq!(poll.resource_type, "message");
        assert_eq!(poll.step_name, "wait_for_reply");
        assert!(poll.intent_desc.is_some());
    }

    #[test]
    fn test_route_listener_default() {
        let listener = RouteListener::default();
        assert_eq!(listener.msg_filter, "creator_last_message");
        assert!(listener.needs_llm);
    }

    #[test]
    fn test_timeout_config_default() {
        let config = TimeoutConfig::default();
        assert_eq!(config.interval_seconds, 43200);
        assert_eq!(config.max_strikes, 3);
    }

    #[test]
    fn test_workflow_result_variants() {
        let cont = WorkflowResult::Continue;
        assert!(matches!(cont, WorkflowResult::Continue));

        let waiting = WorkflowResult::Waiting(vec![]);
        assert!(matches!(waiting, WorkflowResult::Waiting(_)));

        let finished = WorkflowResult::Finished("SUCCESS".to_string(), "done".to_string());
        assert!(matches!(finished, WorkflowResult::Finished(_, _)));
    }
}
