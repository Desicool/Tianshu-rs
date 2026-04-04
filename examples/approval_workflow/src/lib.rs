pub mod workflows;

use workflow_engine::WorkflowRegistry;
use workflows::ApprovalWorkflow;

/// Register all workflows into the registry.
pub fn register_workflows(registry: &mut WorkflowRegistry) {
    registry.register("approval", |_case| Box::new(ApprovalWorkflow));
}
