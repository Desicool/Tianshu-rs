pub mod stages;
pub mod workflows;

use std::collections::HashMap;
use std::sync::Arc;

use example_tools::{
    ListDirectoryTool, ReadFileTool, SearchFilesTool, ShellCommandTool, ToolRegistry, WriteFileTool,
};
use workflow_engine::{llm::LlmProvider, stage::StageBase, WorkflowRegistry};

use stages::{
    ExecuteConcurrentStage, ExecuteExclusiveStage, FeedResultsStage, PlanToolsStage,
    SynthesizeStage,
};
use workflows::{OrchStage, ToolOrchestrationWorkflow};

/// Register all workflows provided by this crate into `registry`.
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

    registry.register("tool_orchestration", move |_case| {
        let mut stage_map: HashMap<OrchStage, Box<dyn StageBase<OrchStage>>> = HashMap::new();

        stage_map.insert(
            OrchStage::PlanTools,
            Box::new(PlanToolsStage {
                llm: Arc::clone(&llm),
                model: model.clone(),
                tools: Arc::clone(&tools),
            }),
        );
        stage_map.insert(
            OrchStage::ExecuteConcurrent,
            Box::new(ExecuteConcurrentStage {
                tools: Arc::clone(&tools),
            }),
        );
        stage_map.insert(
            OrchStage::ExecuteExclusive,
            Box::new(ExecuteExclusiveStage {
                tools: Arc::clone(&tools),
            }),
        );
        stage_map.insert(OrchStage::FeedResults, Box::new(FeedResultsStage));
        stage_map.insert(
            OrchStage::Synthesize,
            Box::new(SynthesizeStage {
                llm: Arc::clone(&llm),
                model: model.clone(),
            }),
        );

        Box::new(ToolOrchestrationWorkflow {
            stages: Arc::new(stage_map),
            start_stage: OrchStage::PlanTools,
        })
    });
}
