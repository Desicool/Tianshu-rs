# Concept: WorkflowContext

**Source**: `crates/workflow_engine/src/context.rs`

## Responsibility
The per-tick execution handle passed to every workflow and stage. Provides: idempotent step checkpointing, mutable state variables, session-scoped (cross-case) state, child workflow spawning, LLM tool-use steps, managed conversation compaction, and observability hooks.

## Boundary
- Created fresh for each tick — **not a long-lived object**
- Bridges the workflow to `CaseStore` and `StateStore` without coupling the workflow to any specific database implementation
- Does not know about routing, scheduling, or the registry

## Key API Surface

### Checkpoints (idempotent step results)
- `step(name, async fn)` — run once; on restart returns cached result
- `step_with_retry(name, policy, async fn)` — like `step` but with retry policy applied to the closure
- `tool_step(name, llm, tools, request, config)` — tool-use loop with automatic checkpointing
- `get_checkpoint(name)` / `save_checkpoint(name, value)` / `clear_step(name)` / `clear_steps(names)` — manual checkpoint control

### State Variables (mutable, non-idempotent)
- `get_state<T>(name, default)` / `set_state<T>(name, value)` — case-scoped key/value state

### Session State (cross-case)
- `get_session_state<T>(name, default)` / `set_session_state<T>(name, value)` — shared across all cases in the same session; **no engine-level locking**, last write wins

### Child Workflows
- `spawn_child(config)` / `spawn_children(configs)` — create child cases immediately in Running state
- `child_status(handle)` — check one child
- `await_children(handles)` — `Pending(n)` while any are in-flight, `AllDone(statuses)` when all finish

### Lifecycle
- `finish(type, description)` — persist final state, clean up StateStore, emit observability record

### Observability
- `set_observer(observer)` — attach before any steps run
- `step_records()` — accumulated step records for this tick

### Managed Conversation
- `set_managed_conversation(conv)` / `managed_conversation()` — opt-in context compaction

## Design Notes
- Checkpoint keys are namespaced as `wf_<step_name>` in StateStore; state var keys as `wf_state_<name>`; session state as `wf_sess_state_<name>`
- A JSON null sentinel marks "cleared" checkpoints (StateStore has no per-key delete by default)
- `finish()` calls `state_store.delete_by_case()` to clean up all runtime state; cleanup errors are logged but do not fail the workflow

## Design History
_No changes yet._
