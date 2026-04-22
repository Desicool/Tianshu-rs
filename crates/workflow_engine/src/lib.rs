// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

pub mod agent;
pub mod agent_context;
pub mod agent_workflow;
pub mod case;
pub mod compact;
pub mod context;
pub mod engine;
pub mod llm;
pub mod llm_resilient;
pub mod observe;
pub mod poll;
pub mod registry;
pub mod retry;
pub mod session;
pub mod spawn;
pub mod stage;
pub mod store;
pub mod token;
pub mod tool;
pub mod tool_loop;
pub mod workflow;

// Re-exports
pub use agent::{Agent, AgentId, Capabilities, CapabilityRestriction};
pub use agent_context::{AgentContext, AgentHandle, AgentSpawnConfig};
pub use agent_workflow::{AgentOutcome, AgentWorkflow, BaseAgent};
pub use case::{make_case, Case, ExecutionState};
pub use compact::{
    CompactionStrategy, LlmSummaryCompaction, ManagedConversation, TruncationCompaction,
};
pub use context::WorkflowContext;
pub use engine::{
    shutdown_signal, ExecutionMode, SchedulerEnvironment, SchedulerV2, ShutdownSignal,
    ShutdownTrigger, TickResult,
};
pub use llm::{
    LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmStreamEvent, LlmTool, LlmUsage,
    StreamingLlmProvider, ToolCall, ToolResult,
};
pub use llm_resilient::ResilientLlmProvider;
pub use observe::{
    observe_llm_call, AgentCompleteRecord, AgentMessageRecord, AgentSpawnRecord, LlmCallRecord,
    ObservedLlmProvider, Observer, ProbeOutcome, ProbeRecord, RetryRecord, StepRecord,
    ToolCallRecord, WorkflowRecord,
};
pub use poll::{
    AgentMessageFetcher, CompositeResourceFetcher, IntentRouterV2, PollEvaluator, PollMatch,
    ResourceFetcher,
};
pub use registry::WorkflowRegistry;
pub use retry::{with_retry, ErrorClass, RetryContext, RetryPolicy};
pub use session::Session;
pub use spawn::{ChildHandle, ChildStatus, ChildrenResult, SpawnConfig};
pub use stage::{run_stages, StageBase, StageKey, StageOutcome};
pub use store::{
    AgentMessage, AgentMessageStore, AgentStore, CaseFilter, CaseStore, InMemoryAgentMessageStore,
    InMemoryAgentStore, InMemoryCaseStore, InMemorySessionStore, InMemoryStateStore,
    SessionStateEntry, SessionStore, StateEntry, StateStore,
};
pub use token::{CharTokenCounter, ContextConfig, TokenCounter};
pub use tool::{ScopedToolRegistry, Tool, ToolRegistry, ToolSafety};
pub use tool_loop::{run_agent_tool_loop, run_tool_loop, ToolLoopConfig, ToolLoopResult};
pub use workflow::{BaseWorkflow, PollPredicate, WorkflowResult};
