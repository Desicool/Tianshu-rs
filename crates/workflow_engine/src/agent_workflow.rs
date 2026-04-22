// Copyright 2026 Desicool
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::agent::Agent;
use crate::agent_context::AgentContext;
use crate::context::WorkflowContext;
use crate::registry::WorkflowRegistry;
use crate::store::{AgentMessageStore, AgentStore};
use crate::tool::ToolRegistry;
use crate::workflow::{BaseWorkflow, WorkflowResult};

/// Return type for `BaseAgent::act()`. Alias of `WorkflowResult` — no conversion needed.
pub type AgentOutcome = WorkflowResult;

/// Type alias for the factory closure stored inside `AgentWorkflow`.
type AgentFactory = dyn Fn(&Agent) -> Box<dyn BaseAgent> + Send + Sync;

/// Trait for agent implementations. Implement this to define agent behavior.
#[async_trait]
pub trait BaseAgent: Send + Sync {
    async fn act(&self, ctx: &mut AgentContext<'_>) -> Result<AgentOutcome>;
}

/// Bridges `BaseAgent` → `BaseWorkflow` so agents are scheduled by `SchedulerV2`.
pub struct AgentWorkflow {
    tool_registry: Arc<ToolRegistry>,
    agent_store: Arc<dyn AgentStore>,
    message_store: Arc<dyn AgentMessageStore>,
    factory: Arc<AgentFactory>,
}

impl AgentWorkflow {
    /// Register an agent workflow in the `WorkflowRegistry`.
    ///
    /// `workflow_code` is the code used in `Case.workflow_code` for this agent role.
    /// `factory` is called once per tick to produce a `BaseAgent` implementation.
    pub fn register(
        registry: &mut WorkflowRegistry,
        workflow_code: &str,
        tool_registry: Arc<ToolRegistry>,
        agent_store: Arc<dyn AgentStore>,
        message_store: Arc<dyn AgentMessageStore>,
        factory: impl Fn(&Agent) -> Box<dyn BaseAgent> + Send + Sync + 'static,
    ) {
        let wf = Arc::new(AgentWorkflow {
            tool_registry,
            agent_store,
            message_store,
            factory: Arc::new(factory),
        });
        registry.register(workflow_code, move |_case| {
            Box::new(AgentWorkflowInstance { inner: wf.clone() }) as Box<dyn BaseWorkflow>
        });
    }
}

/// Thin wrapper so we can hand out per-tick instances without cloning the factory.
struct AgentWorkflowInstance {
    inner: Arc<AgentWorkflow>,
}

#[async_trait]
impl BaseWorkflow for AgentWorkflowInstance {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
        let agent_id_str = ctx.case.case_key.clone();

        // Load agent record from store
        let agent = self
            .inner
            .agent_store
            .get(&crate::agent::AgentId::new(&agent_id_str))
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("agent record not found for case_key={}", agent_id_str)
            })?;

        // Produce the concrete BaseAgent impl
        let agent_impl = (self.inner.factory)(&agent);

        // Build AgentContext borrowing ctx
        let mut agent_ctx = AgentContext::new(
            ctx,
            agent,
            self.inner.tool_registry.clone(),
            self.inner.message_store.clone(),
            self.inner.agent_store.clone(),
        );

        // Execute one tick
        let result = agent_impl.act(&mut agent_ctx).await?;

        Ok(result)
    }
}
