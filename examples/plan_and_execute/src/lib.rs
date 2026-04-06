pub mod workflows;

use std::sync::Arc;

use example_tools::{
    ListDirectoryTool, ReadFileTool, SearchFilesTool, ShellCommandTool, ToolRegistry, WriteFileTool,
};
use workflow_engine::{llm::LlmProvider, WorkflowRegistry};
use workflows::PlanAndExecuteWorkflow;

/// Register all workflows provided by this crate into `registry`.
///
/// The `llm` and `model` are shared across all workflow instances created by
/// the factory closure.
pub fn register_workflows(
    registry: &mut WorkflowRegistry,
    llm: Arc<dyn LlmProvider>,
    model: String,
) {
    // Build the tool registry once and share it across all workflow instances.
    let mut tools = ToolRegistry::new();
    tools.register(ReadFileTool);
    tools.register(WriteFileTool);
    tools.register(ShellCommandTool::new());
    tools.register(SearchFilesTool);
    tools.register(ListDirectoryTool);
    let tools = Arc::new(tools);

    registry.register("plan_and_execute", move |_case| {
        Box::new(PlanAndExecuteWorkflow {
            llm: llm.clone(),
            model: model.clone(),
            tools: Arc::clone(&tools),
        })
    });
}
