//! Multi-agent swarm example: coordinator/worker pattern using the Tianshu
//! workflow engine.
//!
//! Two workflow types share a session:
//! - `"swarm_leader"`: plans tasks, assigns them to workers, waits for results,
//!   then synthesizes a final answer.
//! - `"swarm_worker"`: reads a single task from session state, executes it with
//!   LLM + tools, and reports the result back.

pub mod workflows;

pub use workflows::leader::SubTask;
pub use workflows::{LeaderWorkflow, WorkerWorkflow};

use std::sync::Arc;

use example_tools::ToolRegistry;
use workflow_engine::{llm::LlmProvider, WorkflowRegistry};

/// Register the `"swarm_leader"` and `"swarm_worker"` workflows into `registry`.
pub fn register_workflows(
    registry: &mut WorkflowRegistry,
    llm: Arc<dyn LlmProvider>,
    model: String,
) {
    // Build shared tool registry for workers.
    let tools = Arc::new(ToolRegistry::new());

    let llm_leader = Arc::clone(&llm);
    let model_leader = model.clone();
    registry.register("swarm_leader", move |_case| {
        Box::new(LeaderWorkflow {
            llm: Arc::clone(&llm_leader),
            model: model_leader.clone(),
        })
    });

    let llm_worker = Arc::clone(&llm);
    let model_worker = model.clone();
    let tools_worker = Arc::clone(&tools);
    registry.register("swarm_worker", move |_case| {
        Box::new(WorkerWorkflow {
            llm: Arc::clone(&llm_worker),
            model: model_worker.clone(),
            tools: Arc::clone(&tools_worker),
        })
    });
}
