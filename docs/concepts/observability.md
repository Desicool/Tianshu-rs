# Concept: Observability

**Source**: `crates/workflow_engine_observe/`

## Responsibility
Collects structured records of what happens during workflow execution: individual step results, LLM API calls (with token usage), tool invocations, and workflow completion summaries. Provides pluggable observer implementations and dataset export utilities.

## Boundary
- Observers are **side-effect-only** — they must never influence workflow execution, routing, or scheduling
- The engine calls observer methods fire-and-forget: `on_step()`, `on_workflow_complete()`, `flush()`
- Observer injection is opt-in via `WorkflowContext::set_observer()` — workflows do not depend on observability being present

## Key Types
- `Observer` (trait) — `async fn on_step(record)` / `async fn on_workflow_complete(record)` / `async fn flush()`
- `InMemoryObserver` — accumulates records in memory; useful for tests and in-process inspection
- `JsonlObserver` — streams records as newline-delimited JSON to a file or writer
- `CompositeObserver` — fans out to multiple observers simultaneously
- `StepRecord` — `{ case_key, workflow_code, step_name, input_resource_data, output, duration_ms, cached, error, timestamp }`
- `LlmCallRecord` — `{ model, input_tokens, output_tokens, duration_ms, ... }`
- `ToolCallRecord` — `{ tool_name, arguments, result, duration_ms }`
- `WorkflowRecord` — `{ case_key, session_id, workflow_code, steps, total_duration_ms, finished_type, ... }`
- Dataset helpers: `llm_dataset()`, `step_dataset()`, `workflow_dataset()` — for analytics export

## Design Notes
- `flush()` must be called before process exit to ensure all buffered records are written — `WorkflowContext::finish()` calls it automatically
- `CompositeObserver` allows pairing `InMemoryObserver` (for tests) with `JsonlObserver` (for production logging) without changing workflow code

## Design History
_No changes yet._
