pub mod workflows;

use std::sync::Arc;

use example_tools::{
    ListDirectoryTool, ReadFileTool, SearchFilesTool, ShellCommandTool, ToolRegistry, WriteFileTool,
};
use workflow_engine::{llm::LlmProvider, WorkflowRegistry};
use workflows::ConversationWorkflow;

/// Register all workflows provided by this crate into `registry`.
///
/// Registers a `"conversation"` workflow backed by standard file tools.
pub fn register_workflows(
    registry: &mut WorkflowRegistry,
    llm: Arc<dyn LlmProvider>,
    model: String,
) {
    let mut tools = ToolRegistry::new();
    tools.register(ReadFileTool);
    tools.register(WriteFileTool);
    tools.register(ShellCommandTool::new());
    tools.register(SearchFilesTool);
    tools.register(ListDirectoryTool);
    let tools = Arc::new(tools);

    let system_prompt = "You are a helpful conversational assistant. \
        You remember the conversation history and use available tools when needed. \
        Be concise, friendly, and accurate."
        .to_string();

    registry.register("conversation", move |_case| {
        Box::new(ConversationWorkflow {
            llm: Arc::clone(&llm),
            model: model.clone(),
            tools: Arc::clone(&tools),
            system_prompt: system_prompt.clone(),
        })
    });
}
