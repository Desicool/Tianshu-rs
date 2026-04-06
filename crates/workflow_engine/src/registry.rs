// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use crate::case::Case;
use crate::workflow::BaseWorkflow;

/// Type alias for workflow factory functions.
type WorkflowFactory = Box<dyn Fn(Case) -> Box<dyn BaseWorkflow> + Send + Sync>;

/// Registry that maps workflow code strings to factory functions.
pub struct WorkflowRegistry {
    factories: HashMap<String, WorkflowFactory>,
}

impl WorkflowRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a factory function for a given workflow code.
    /// Replaces any previously registered factory for that code.
    pub fn register<F>(&mut self, code: &str, factory: F)
    where
        F: Fn(Case) -> Box<dyn BaseWorkflow> + Send + Sync + 'static,
    {
        self.factories.insert(code.to_string(), Box::new(factory));
    }

    /// Look up a workflow code and invoke its factory with the provided Case.
    pub fn get(&self, code: &str, case: Case) -> Option<Box<dyn BaseWorkflow>> {
        self.factories.get(code).map(|factory| factory(case))
    }

    /// Return all registered workflow codes, sorted alphabetically.
    pub fn registered_codes(&self) -> Vec<String> {
        let mut codes: Vec<String> = self.factories.keys().cloned().collect();
        codes.sort();
        codes
    }
}

impl Default for WorkflowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;

    use crate::context::WorkflowContext;
    use crate::workflow::WorkflowResult;

    struct DummyWorkflow {
        case: Case,
    }

    #[async_trait]
    impl BaseWorkflow for DummyWorkflow {
        async fn run(&self, _ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
            Ok(WorkflowResult::Finished(
                "SUCCESS".into(),
                format!("dummy finished for {}", self.case.case_key),
            ))
        }
    }

    struct AnotherWorkflow;
    #[async_trait]
    impl BaseWorkflow for AnotherWorkflow {
        async fn run(&self, _ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
            Ok(WorkflowResult::Continue)
        }
    }

    fn make_test_case(code: &str) -> Case {
        Case::new("test_case_key".into(), "sess_nil".into(), code.into())
    }

    #[test]
    fn test_new_registry_is_empty() {
        assert!(WorkflowRegistry::new().registered_codes().is_empty());
    }

    #[test]
    fn test_default_registry_is_empty() {
        assert!(WorkflowRegistry::default().registered_codes().is_empty());
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = WorkflowRegistry::new();
        reg.register("code_0", |case: Case| Box::new(DummyWorkflow { case }));
        assert!(reg.get("code_0", make_test_case("code_0")).is_some());
    }

    #[test]
    fn test_get_unknown_code_returns_none() {
        let reg = WorkflowRegistry::new();
        assert!(reg
            .get("nonexistent", make_test_case("nonexistent"))
            .is_none());
    }

    #[test]
    fn test_registered_codes_sorted() {
        let mut reg = WorkflowRegistry::new();
        reg.register("code_2", |case: Case| Box::new(DummyWorkflow { case }));
        reg.register("code_0", |case: Case| Box::new(DummyWorkflow { case }));
        reg.register("code_1", |case: Case| Box::new(DummyWorkflow { case }));
        assert_eq!(reg.registered_codes(), vec!["code_0", "code_1", "code_2"]);
    }

    #[test]
    fn test_register_replaces_existing() {
        let mut reg = WorkflowRegistry::new();
        reg.register("code_0", |case: Case| Box::new(DummyWorkflow { case }));
        reg.register("code_0", |_: Case| Box::new(AnotherWorkflow));
        assert_eq!(reg.registered_codes().len(), 1);
        assert!(reg.get("code_0", make_test_case("code_0")).is_some());
    }

    #[tokio::test]
    async fn test_created_workflow_has_no_route_listener() {
        let mut reg = WorkflowRegistry::new();
        reg.register("code_0", |case: Case| Box::new(DummyWorkflow { case }));

        let wf = reg.get("code_0", make_test_case("code_0")).unwrap();
        assert!(wf.route_listener().is_none());
    }
}
