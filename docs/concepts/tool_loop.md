# Concept: Tool Loop

**Source**: `crates/workflow_engine/src/tool_loop.rs`

## Responsibility
Runs the LLM ↔ tool execution loop: sends a request to the LLM, executes any tool calls the model returns, appends results to the conversation, and repeats until the model produces a final text answer or `max_rounds` is exhausted.

## Boundary
- Invoked only via `WorkflowContext::tool_step()` — never called directly by workflow code
- Does **not** persist state — checkpointing of the final result is handled by `WorkflowContext`
- The tool registry (`ToolRegistry`) is passed in at call time; the tool loop does not own it

## Key Types
- `ToolLoopConfig` — `{ max_rounds: usize, ... }`
- `ToolLoopResult` — `{ final_text: String, rounds: usize, ... }`
- `run_tool_loop(llm, request, tools, config, observer)` — top-level function

## Design Notes
- If `max_rounds` is exhausted without a final answer, `run_tool_loop` returns the last partial text or an error — callers should handle this gracefully
- The observer (if provided) receives `LlmCallRecord` and `ToolCallRecord` events for each round

## Design History
_No changes yet._
