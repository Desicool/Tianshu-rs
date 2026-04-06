use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionState {
    Running,
    Waiting,
    Finished,
}

impl std::fmt::Display for ExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionState::Running => write!(f, "running"),
            ExecutionState::Waiting => write!(f, "waiting"),
            ExecutionState::Finished => write!(f, "finished"),
        }
    }
}

impl ExecutionState {
    pub fn from_str_lowercase(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "waiting" => Some(Self::Waiting),
            "finished" => Some(Self::Finished),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    pub case_key: String,
    pub session_id: String,
    pub workflow_code: String,
    pub execution_state: ExecutionState,
    pub finished_type: Option<String>,
    pub finished_description: Option<String>,
    pub parent_key: Option<String>,
    pub child_keys: Vec<String>,
    pub lifecycle_state: String,
    pub processing_report: Vec<JsonValue>,
    pub resource_data: Option<JsonValue>,
    pub private_vars: Option<JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub depth: u32,
}

impl Case {
    pub fn new(case_key: String, session_id: String, workflow_code: String) -> Self {
        let now = Utc::now();
        Self {
            case_key,
            session_id,
            workflow_code,
            execution_state: ExecutionState::Running,
            finished_type: None,
            finished_description: None,
            parent_key: None,
            child_keys: Vec::new(),
            lifecycle_state: "normal".to_string(),
            processing_report: Vec::new(),
            resource_data: None,
            private_vars: None,
            created_at: now,
            updated_at: now,
            depth: 0,
        }
    }

    pub fn mc_run(&mut self) {
        self.execution_state = ExecutionState::Running;
        self.updated_at = Utc::now();
    }

    pub fn mc_wait(&mut self) {
        self.execution_state = ExecutionState::Waiting;
        self.updated_at = Utc::now();
    }

    pub fn mc_finish(&mut self, finished_type: String, finished_description: String) {
        self.execution_state = ExecutionState::Finished;
        self.finished_type = Some(finished_type);
        self.finished_description = Some(finished_description);
        self.updated_at = Utc::now();
    }

    pub fn is_paused(&self) -> bool {
        self.lifecycle_state == "pause"
    }

    pub fn is_stopped(&self) -> bool {
        self.lifecycle_state == "stop"
    }

    pub fn is_active(&self) -> bool {
        !self.is_paused() && !self.is_stopped() && self.execution_state != ExecutionState::Finished
    }
}

/// Convenience constructor that generates the case_key automatically.
pub fn make_case(
    session_id: String,
    workflow_code: &str,
    case_type: &str,
    description: &str,
    parent_key: Option<String>,
) -> Case {
    let case_key = format!(
        "{}_{}_{}",
        session_id,
        Utc::now().timestamp_millis(),
        workflow_code
    );
    let now = Utc::now();
    Case {
        case_key,
        session_id,
        workflow_code: workflow_code.to_string(),
        execution_state: ExecutionState::Running,
        finished_type: None,
        finished_description: Some(description.to_string()),
        parent_key,
        child_keys: Vec::new(),
        lifecycle_state: "normal".to_string(),
        processing_report: Vec::new(),
        resource_data: None,
        private_vars: Some(serde_json::json!({"case_type": case_type})),
        created_at: now,
        updated_at: now,
        depth: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_state_display() {
        assert_eq!(ExecutionState::Running.to_string(), "running");
        assert_eq!(ExecutionState::Waiting.to_string(), "waiting");
        assert_eq!(ExecutionState::Finished.to_string(), "finished");
    }

    #[test]
    fn execution_state_from_str() {
        assert_eq!(
            ExecutionState::from_str_lowercase("running"),
            Some(ExecutionState::Running)
        );
        assert_eq!(ExecutionState::from_str_lowercase("invalid"), None);
    }

    #[test]
    fn case_new_defaults() {
        let c = Case::new("k".into(), "s".into(), "wf".into());
        assert_eq!(c.execution_state, ExecutionState::Running);
        assert_eq!(c.lifecycle_state, "normal");
        assert!(c.is_active());
    }

    #[test]
    fn case_state_transitions() {
        let mut c = Case::new("k".into(), "s".into(), "wf".into());
        c.mc_wait();
        assert_eq!(c.execution_state, ExecutionState::Waiting);
        c.mc_run();
        assert_eq!(c.execution_state, ExecutionState::Running);
        c.mc_finish("ok".into(), "done".into());
        assert_eq!(c.execution_state, ExecutionState::Finished);
        assert!(!c.is_active());
    }

    #[test]
    fn case_lifecycle_pause_stop() {
        let mut c = Case::new("k".into(), "s".into(), "wf".into());
        c.lifecycle_state = "pause".to_string();
        assert!(c.is_paused());
        assert!(!c.is_active());
        c.lifecycle_state = "stop".to_string();
        assert!(c.is_stopped());
        assert!(!c.is_active());
    }
}
