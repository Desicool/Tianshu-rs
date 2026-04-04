use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use crate::context::WorkflowContext;
use crate::workflow::{PollPredicate, WorkflowResult};

/// Marker trait for stage key enums. Each workflow defines its own enum.
pub trait StageKey: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static {
    fn as_str(&self) -> &'static str;
}

/// Type-safe stage outcome. Invalid stage references are caught at compile time
/// because the stage type parameter `S` must be a valid enum variant.
pub enum StageOutcome<S: StageKey> {
    /// Stage is waiting for an external event
    Waiting(Vec<PollPredicate>),
    /// Stage completed; advance to next stage and optionally clear prior stages
    Next {
        stage: S,
        clear_stages: Vec<S>,
    },
    /// Workflow is finished
    Finish {
        finish_type: String,
        finish_desc: String,
    },
}

impl<S: StageKey> StageOutcome<S> {
    pub fn next(stage: S) -> Self {
        Self::Next {
            stage,
            clear_stages: Vec::new(),
        }
    }

    pub fn next_with_clear(stage: S, clear: Vec<S>) -> Self {
        Self::Next {
            stage,
            clear_stages: clear,
        }
    }

    pub fn finish(t: impl Into<String>, d: impl Into<String>) -> Self {
        Self::Finish {
            finish_type: t.into(),
            finish_desc: d.into(),
        }
    }

    pub fn waiting(polls: Vec<PollPredicate>) -> Self {
        Self::Waiting(polls)
    }
}

/// Async trait for individual workflow stages.
#[async_trait]
pub trait StageBase<S: StageKey>: Send + Sync {
    /// The stage key this stage handles
    fn stage_key(&self) -> S;

    /// Checkpoint step keys owned by this stage (cleared on back-transition)
    fn step_keys(&self) -> &[&str];

    /// Execute this stage and return an outcome
    async fn execute(&self, ctx: &mut WorkflowContext) -> Result<StageOutcome<S>>;

    /// Clear all checkpoints owned by this stage
    async fn clear(&self, ctx: &mut WorkflowContext) -> Result<()> {
        ctx.clear_steps(self.step_keys()).await
    }
}

/// Run a stage-based workflow from `start_stage`, advancing through stages
/// until a `Waiting` or `Finish` outcome is produced.
pub async fn run_stages<S: StageKey>(
    ctx: &mut WorkflowContext,
    start_stage: S,
    registry: &HashMap<S, Box<dyn StageBase<S>>>,
) -> Result<WorkflowResult> {
    let mut current: Option<S> = Some(start_stage);
    while let Some(key) = current {
        let stage = registry
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("No stage registered for key {:?}", key))?;

        match stage.execute(ctx).await? {
            StageOutcome::Waiting(polls) => return Ok(WorkflowResult::Waiting(polls)),
            StageOutcome::Finish {
                finish_type,
                finish_desc,
            } => {
                ctx.finish(finish_type.clone(), finish_desc.clone()).await?;
                return Ok(WorkflowResult::Finished(finish_type, finish_desc));
            }
            StageOutcome::Next {
                stage: next_stage,
                clear_stages,
            } => {
                for s in &clear_stages {
                    if let Some(st) = registry.get(s) {
                        st.clear(ctx).await?;
                    }
                }
                current = Some(next_stage);
            }
        }
    }
    unreachable!("run_stages loop must always return via Waiting or Finish")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::Case;
    use crate::context::WorkflowContext;
    use crate::store::{InMemoryCaseStore, InMemoryStateStore};
    use std::sync::Arc;

    fn make_test_case() -> Case {
        Case::new(
            "test_stage_case".into(),
            "sess_test".into(),
            "test_workflow".into(),
        )
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestStage {
        Start,
        Middle,
        End,
    }

    impl StageKey for TestStage {
        fn as_str(&self) -> &'static str {
            match self {
                Self::Start => "start",
                Self::Middle => "middle",
                Self::End => "end",
            }
        }
    }

    #[test]
    fn test_stage_outcome_next() {
        let outcome: StageOutcome<TestStage> = StageOutcome::next(TestStage::Middle);
        match outcome {
            StageOutcome::Next {
                stage,
                clear_stages,
            } => {
                assert_eq!(stage, TestStage::Middle);
                assert!(clear_stages.is_empty());
            }
            _ => panic!("Expected Next outcome"),
        }
    }

    #[test]
    fn test_stage_outcome_next_with_clear() {
        let outcome: StageOutcome<TestStage> = StageOutcome::next_with_clear(
            TestStage::Start,
            vec![TestStage::Middle, TestStage::End],
        );
        match outcome {
            StageOutcome::Next {
                stage,
                clear_stages,
            } => {
                assert_eq!(stage, TestStage::Start);
                assert_eq!(clear_stages.len(), 2);
            }
            _ => panic!("Expected Next outcome"),
        }
    }

    #[test]
    fn test_stage_outcome_finish() {
        let outcome: StageOutcome<TestStage> = StageOutcome::finish("success", "done");
        match outcome {
            StageOutcome::Finish {
                finish_type,
                finish_desc,
            } => {
                assert_eq!(finish_type, "success");
                assert_eq!(finish_desc, "done");
            }
            _ => panic!("Expected Finish outcome"),
        }
    }

    #[test]
    fn test_stage_outcome_waiting() {
        let poll = PollPredicate {
            resource_type: "message".to_string(),
            resource_id: "creator_123".to_string(),
            step_name: "wait_msg".to_string(),
            intent_desc: None,
        };
        let outcome: StageOutcome<TestStage> = StageOutcome::waiting(vec![poll]);
        match outcome {
            StageOutcome::Waiting(polls) => {
                assert_eq!(polls.len(), 1);
                assert_eq!(polls[0].step_name, "wait_msg");
            }
            _ => panic!("Expected Waiting outcome"),
        }
    }

    #[test]
    fn test_stage_key_as_str() {
        assert_eq!(TestStage::Start.as_str(), "start");
        assert_eq!(TestStage::Middle.as_str(), "middle");
        assert_eq!(TestStage::End.as_str(), "end");
    }

    #[test]
    fn test_context_creation_for_stages() {
        let case = make_test_case();
        let ctx = WorkflowContext::new(
            case,
            Arc::new(InMemoryCaseStore::default()),
            Arc::new(InMemoryStateStore::default()),
        );
        assert_eq!(ctx.case.case_key, "test_stage_case");
    }

    #[tokio::test]
    async fn test_run_stages_unknown_stage_returns_error() {
        let case = make_test_case();
        let mut ctx = WorkflowContext::new(
            case,
            Arc::new(InMemoryCaseStore::default()),
            Arc::new(InMemoryStateStore::default()),
        );
        let registry: HashMap<TestStage, Box<dyn StageBase<TestStage>>> = HashMap::new();

        let result = run_stages(&mut ctx, TestStage::Start, &registry).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No stage registered"));
    }

    struct WaitingStage;

    #[async_trait]
    impl StageBase<TestStage> for WaitingStage {
        fn stage_key(&self) -> TestStage {
            TestStage::Start
        }
        fn step_keys(&self) -> &[&str] {
            &[]
        }
        async fn execute(&self, _ctx: &mut WorkflowContext) -> Result<StageOutcome<TestStage>> {
            let poll = PollPredicate {
                resource_type: "message".to_string(),
                resource_id: "creator_id".to_string(),
                step_name: "wait_step".to_string(),
                intent_desc: None,
            };
            Ok(StageOutcome::waiting(vec![poll]))
        }
    }

    #[tokio::test]
    async fn test_run_stages_waiting_returns_immediately() {
        let case = make_test_case();
        let mut ctx = WorkflowContext::new(
            case,
            Arc::new(InMemoryCaseStore::default()),
            Arc::new(InMemoryStateStore::default()),
        );
        let mut registry: HashMap<TestStage, Box<dyn StageBase<TestStage>>> = HashMap::new();
        registry.insert(TestStage::Start, Box::new(WaitingStage));

        let result = run_stages(&mut ctx, TestStage::Start, &registry).await.unwrap();
        assert!(matches!(result, WorkflowResult::Waiting(_)));
    }
}
