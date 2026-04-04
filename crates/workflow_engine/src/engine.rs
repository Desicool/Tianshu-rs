use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::case::{Case, ExecutionState};
use crate::context::WorkflowContext;
use crate::poll::{PollEvaluator, ResourceFetcher};
use crate::registry::WorkflowRegistry;
use crate::store::{CaseStore, StateStore};
use crate::workflow::{PollPredicate, WorkflowResult};

/// Collected poll information from a waiting workflow probe.
struct WaitingProbeResult {
    case_key: String,
    polls: Vec<PollPredicate>,
}

/// Simple in-memory environment for one scheduler tick.
pub struct SchedulerEnvironment {
    pub session_id: String,
    pub current_case_dict: HashMap<String, Case>,
}

impl SchedulerEnvironment {
    pub fn new(session_id: impl Into<String>, cases: Vec<Case>) -> Self {
        let mut dict = HashMap::new();
        for c in cases {
            dict.insert(c.case_key.clone(), c);
        }
        Self {
            session_id: session_id.into(),
            current_case_dict: dict,
        }
    }

    pub fn delete_runtime_vl_by_case_key(&mut self, _case_key: &str) {
        // State cleanup happens inside WorkflowContext::finish via StateStore.
        // Nothing to do here for the in-process dictionary.
    }
}

/// SchedulerV2 drives workflow execution each tick.
///
/// Mirrors the Python SchedulerV2: separates cases by execution state,
/// probes waiting workflows for poll predicates, and executes ready ones.
pub struct SchedulerV2 {
    /// Tracks which case_keys were woken up during the current tick
    /// to avoid duplicate execution.
    woken_keys: Vec<String>,
}

impl SchedulerV2 {
    pub fn new() -> Self {
        Self {
            woken_keys: Vec::new(),
        }
    }

    /// Run one scheduler tick across all cases in the environment.
    ///
    /// Returns `true` if any workflow was executed, `false` otherwise.
    ///
    /// The tick proceeds in phases:
    /// 1. Partition active cases into running vs waiting
    /// 2. Probe waiting cases to collect poll predicates
    /// 3. Evaluate polls to determine which waiting cases should wake up
    /// 4. Execute all ready workflows (running + newly woken)
    pub async fn tick(
        &mut self,
        env: &mut SchedulerEnvironment,
        registry: &WorkflowRegistry,
        case_store: Arc<dyn CaseStore>,
        state_store: Arc<dyn StateStore>,
        fetcher: Option<&dyn ResourceFetcher>,
    ) -> Result<bool> {
        self.woken_keys.clear();

        if env.current_case_dict.is_empty() {
            return Ok(false);
        }

        // Phase 1: Partition cases by execution state.
        let mut running_keys: Vec<String> = Vec::new();
        let mut waiting_keys: Vec<String> = Vec::new();

        for (case_key, case) in env.current_case_dict.iter() {
            if !case.is_active() {
                continue;
            }
            match case.execution_state {
                ExecutionState::Running => running_keys.push(case_key.clone()),
                ExecutionState::Waiting => waiting_keys.push(case_key.clone()),
                ExecutionState::Finished => {}
            }
        }

        let mut any_executed = false;

        // Phase 2: Probe waiting cases to gather poll predicates.
        let mut probe_results: Vec<WaitingProbeResult> = Vec::new();

        for case_key in &waiting_keys {
            let case = match env.current_case_dict.get(case_key) {
                Some(c) => c.clone(),
                None => continue,
            };

            let wf = match registry.get(&case.workflow_code, case.clone()) {
                Some(w) => w,
                None => {
                    warn!(
                        "No workflow registered for code='{}', case_key='{}'",
                        case.workflow_code, case_key
                    );
                    continue;
                }
            };

            let mut ctx = WorkflowContext::new(
                case,
                Arc::clone(&case_store),
                Arc::clone(&state_store),
            );

            match wf.run(&mut ctx).await {
                Ok(WorkflowResult::Waiting(polls)) => {
                    if !polls.is_empty() {
                        probe_results.push(WaitingProbeResult {
                            case_key: case_key.clone(),
                            polls,
                        });
                    }
                }
                Ok(WorkflowResult::Continue) => {
                    info!(
                        "Waiting workflow returned Continue, waking case_key='{}'",
                        case_key
                    );
                    if let Some(case_mut) = env.current_case_dict.get_mut(case_key) {
                        case_mut.mc_run();
                    }
                    self.woken_keys.push(case_key.clone());
                }
                Ok(WorkflowResult::Finished(ft, fd)) => {
                    info!(
                        "Waiting workflow finished during probe: case_key='{}', type='{}'",
                        case_key, ft
                    );
                    if let Some(case_mut) = env.current_case_dict.get_mut(case_key) {
                        case_mut.mc_finish(ft, fd);
                    }
                }
                Err(e) => {
                    error!("Workflow probe error for case_key='{}': {}", case_key, e);
                }
            }
        }

        // Phase 3: Evaluate poll predicates.
        let mut data_polls: Vec<(String, PollPredicate)> = Vec::new();
        let mut router_polls: Vec<(String, PollPredicate)> = Vec::new();

        for result in &probe_results {
            for poll in &result.polls {
                if poll.intent_desc.is_some() {
                    router_polls.push((result.case_key.clone(), poll.clone()));
                } else {
                    data_polls.push((result.case_key.clone(), poll.clone()));
                }
            }
        }

        let evaluator = PollEvaluator::new();

        if let Some(fetcher) = fetcher {
            for (case_key, poll) in data_polls.iter().chain(router_polls.iter()) {
                let matches = evaluator.evaluate(&[poll.clone()], fetcher).await;
                if !matches.is_empty() {
                    info!(
                        "Poll matched: case_key='{}', step='{}'",
                        case_key, poll.step_name
                    );
                    if let Some(case_mut) = env.current_case_dict.get_mut(case_key) {
                        case_mut.mc_run();
                    }
                    if !self.woken_keys.contains(case_key) {
                        self.woken_keys.push(case_key.clone());
                    }
                }
            }
        }

        // Phase 4: Execute ready workflows (running + woken-up).
        let mut ready_keys: Vec<String> = running_keys;
        for key in &self.woken_keys {
            if !ready_keys.contains(key) {
                ready_keys.push(key.clone());
            }
        }

        for case_key in &ready_keys {
            let case = match env.current_case_dict.get(case_key) {
                Some(c) => c.clone(),
                None => continue,
            };

            let wf = match registry.get(&case.workflow_code, case.clone()) {
                Some(w) => w,
                None => {
                    warn!(
                        "No workflow registered for code='{}', case_key='{}'",
                        case.workflow_code, case_key
                    );
                    continue;
                }
            };

            let mut ctx = WorkflowContext::new(
                case,
                Arc::clone(&case_store),
                Arc::clone(&state_store),
            );

            match wf.run(&mut ctx).await {
                Ok(WorkflowResult::Continue) => {
                    info!("Workflow step completed: case_key='{}'", case_key);
                    any_executed = true;
                }
                Ok(WorkflowResult::Waiting(polls)) => {
                    info!(
                        "Workflow entered wait state: case_key='{}', polls={}",
                        case_key,
                        polls.len()
                    );
                    if let Some(case_mut) = env.current_case_dict.get_mut(case_key) {
                        case_mut.mc_wait();
                    }
                    any_executed = true;
                }
                Ok(WorkflowResult::Finished(ft, fd)) => {
                    info!(
                        "Workflow finished: case_key='{}', type='{}', desc='{}'",
                        case_key, ft, fd
                    );
                    if let Some(case_mut) = env.current_case_dict.get_mut(case_key) {
                        case_mut.mc_finish(ft.clone(), fd.clone());
                    }
                    env.delete_runtime_vl_by_case_key(case_key);
                    any_executed = true;
                }
                Err(e) => {
                    error!(
                        "Workflow execution error for case_key='{}': {}",
                        case_key, e
                    );
                }
            }
        }

        Ok(any_executed)
    }
}

impl Default for SchedulerV2 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{InMemoryCaseStore, InMemoryStateStore};
    use crate::workflow::{BaseWorkflow, PollPredicate, WorkflowResult};
    use async_trait::async_trait;

    fn make_test_case(code: &str, state: ExecutionState) -> Case {
        let mut c = Case::new(
            format!("case_{}", code),
            "sess_test".into(),
            code.into(),
        );
        match state {
            ExecutionState::Running => {} // default
            ExecutionState::Waiting => c.mc_wait(),
            ExecutionState::Finished => c.mc_finish("ok".into(), "done".into()),
        }
        c
    }

    fn make_env(cases: Vec<Case>) -> SchedulerEnvironment {
        SchedulerEnvironment::new("sess_test", cases)
    }

    fn make_stores() -> (Arc<dyn CaseStore>, Arc<dyn StateStore>) {
        (
            Arc::new(InMemoryCaseStore::default()),
            Arc::new(InMemoryStateStore::default()),
        )
    }

    struct ContinueWorkflow;
    #[async_trait]
    impl BaseWorkflow for ContinueWorkflow {
        async fn run(&self, _ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
            Ok(WorkflowResult::Continue)
        }
    }

    struct FinishWorkflow;
    #[async_trait]
    impl BaseWorkflow for FinishWorkflow {
        async fn run(&self, _ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
            Ok(WorkflowResult::Finished("SUCCESS".into(), "All done".into()))
        }
    }

    struct WaitingWorkflow;
    #[async_trait]
    impl BaseWorkflow for WaitingWorkflow {
        async fn run(&self, _ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
            Ok(WorkflowResult::Waiting(vec![PollPredicate {
                resource_type: "message".into(),
                resource_id: "creator_123".into(),
                step_name: "wait_reply".into(),
                intent_desc: None,
            }]))
        }
    }

    struct ErrorWorkflow;
    #[async_trait]
    impl BaseWorkflow for ErrorWorkflow {
        async fn run(&self, _ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
            Err(anyhow::anyhow!("workflow exploded"))
        }
    }

    fn registry_with(entries: Vec<(&str, &str)>) -> WorkflowRegistry {
        let mut reg = WorkflowRegistry::new();
        for (code, behavior) in entries {
            match behavior {
                "continue" => reg.register(code, |_c: Case| Box::new(ContinueWorkflow)),
                "finish" => reg.register(code, |_c: Case| Box::new(FinishWorkflow)),
                "waiting" => reg.register(code, |_c: Case| Box::new(WaitingWorkflow)),
                "error" => reg.register(code, |_c: Case| Box::new(ErrorWorkflow)),
                _ => panic!("unknown behavior: {}", behavior),
            }
        }
        reg
    }

    #[tokio::test]
    async fn test_tick_empty_case_dict_returns_false() {
        let mut scheduler = SchedulerV2::new();
        let mut env = make_env(vec![]);
        let registry = WorkflowRegistry::new();
        let (cs, ss) = make_stores();

        let result = scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_tick_running_workflow_executes() {
        let mut scheduler = SchedulerV2::new();
        let case = make_test_case("greeting", ExecutionState::Running);
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("greeting", "continue")]);
        let (cs, ss) = make_stores();

        let result = scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_tick_finished_workflow_not_executed() {
        let mut scheduler = SchedulerV2::new();
        let case = make_test_case("greeting", ExecutionState::Finished);
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("greeting", "continue")]);
        let (cs, ss) = make_stores();

        let result = scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_tick_paused_case_skipped() {
        let mut scheduler = SchedulerV2::new();
        let mut case = make_test_case("greeting", ExecutionState::Running);
        case.lifecycle_state = "pause".into();
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("greeting", "continue")]);
        let (cs, ss) = make_stores();

        let result = scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_tick_stopped_case_skipped() {
        let mut scheduler = SchedulerV2::new();
        let mut case = make_test_case("greeting", ExecutionState::Running);
        case.lifecycle_state = "stop".into();
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("greeting", "continue")]);
        let (cs, ss) = make_stores();

        let result = scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_tick_unknown_workflow_code_skipped() {
        let mut scheduler = SchedulerV2::new();
        let case = make_test_case("unknown_wf", ExecutionState::Running);
        let mut env = make_env(vec![case]);
        let registry = WorkflowRegistry::new();
        let (cs, ss) = make_stores();

        let result = scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_tick_workflow_finishes_updates_case() {
        let mut scheduler = SchedulerV2::new();
        let case = make_test_case("checkout", ExecutionState::Running);
        let case_key = case.case_key.clone();
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("checkout", "finish")]);
        let (cs, ss) = make_stores();

        scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();

        let updated = env.current_case_dict.get(&case_key).unwrap();
        assert_eq!(updated.execution_state, ExecutionState::Finished);
        assert_eq!(updated.finished_type.as_deref(), Some("SUCCESS"));
    }

    #[tokio::test]
    async fn test_tick_workflow_enters_waiting_updates_case() {
        let mut scheduler = SchedulerV2::new();
        let case = make_test_case("poll_wf", ExecutionState::Running);
        let case_key = case.case_key.clone();
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("poll_wf", "waiting")]);
        let (cs, ss) = make_stores();

        scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();

        let updated = env.current_case_dict.get(&case_key).unwrap();
        assert_eq!(updated.execution_state, ExecutionState::Waiting);
    }

    #[tokio::test]
    async fn test_tick_workflow_error_does_not_crash() {
        let mut scheduler = SchedulerV2::new();
        let case = make_test_case("bad_wf", ExecutionState::Running);
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("bad_wf", "error")]);
        let (cs, ss) = make_stores();

        let result = scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_tick_waiting_probe_returns_continue_wakes_up() {
        let mut scheduler = SchedulerV2::new();
        let case = make_test_case("wake_wf", ExecutionState::Waiting);
        let case_key = case.case_key.clone();
        let mut env = make_env(vec![case]);
        let registry = registry_with(vec![("wake_wf", "continue")]);
        let (cs, ss) = make_stores();

        scheduler.tick(&mut env, &registry, cs, ss, None).await.unwrap();

        let updated = env.current_case_dict.get(&case_key).unwrap();
        assert_eq!(updated.execution_state, ExecutionState::Running);
    }

    #[tokio::test]
    async fn test_scheduler_default_trait() {
        let scheduler = SchedulerV2::default();
        assert!(scheduler.woken_keys.is_empty());
    }
}
