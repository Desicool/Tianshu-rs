# Concept: LLM Abstraction

**Source**: `crates/workflow_engine/src/llm.rs`, `crates/workflow_engine/src/llm_resilient.rs`

## Responsibility
Provider-agnostic LLM interface. Decouples the core engine from any specific LLM vendor. Concrete providers are injected at startup by user code.

## Boundary
- The core engine (`workflow_engine` crate) only depends on these traits — never on a concrete provider
- Concrete implementations (OpenAI, etc.) live in separate crates (`crates/workflow_engine_llm_openai`)
- `ResilientLlmProvider` is an engine-provided decorator — it wraps any `LlmProvider` with retry logic without knowing the underlying vendor

## Key Types
- `LlmProvider` (trait) — `async fn complete(request: LlmRequest) -> Result<LlmResponse>`
- `StreamingLlmProvider` (trait) — streaming variant; yields `LlmStreamEvent`
- `LlmRequest` — `{ messages: Vec<LlmMessage>, tools: Vec<LlmTool>, model, temperature, max_tokens, ... }`
- `LlmResponse` — `{ content: String, tool_calls: Vec<ToolCall>, usage: LlmUsage, ... }`
- `LlmMessage` — `{ role, content }` (system / user / assistant / tool)
- `LlmTool` — tool schema for function calling
- `ToolCall` — `{ id, name, arguments: JsonValue }`
- `ToolResult` — `{ tool_call_id, content }`
- `LlmUsage` — `{ input_tokens, output_tokens }`
- `ResilientLlmProvider` — wraps any `LlmProvider`; configured via `RetryPolicy`

## Design Notes
- `ResilientLlmProvider` uses the same `RetryPolicy` type as `WorkflowContext::step_with_retry` — consistent retry semantics across the engine

## Design History
_No changes yet._
