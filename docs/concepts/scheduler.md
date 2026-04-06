# Concept: Scheduler

**Source**: `crates/workflow_engine/src/engine.rs`

## Responsibility
Drives the execution loop. Each tick: selects all Running cases from CaseStore, dispatches each to its registered workflow via WorkflowRegistry, updates ExecutionState based on WorkflowResult, and handles graceful shutdown.

## Boundary
- Knows only `ExecutionState` (Running / Waiting / Finished) and `TickResult` — nothing about what a workflow does internally
- Must not contain business logic or domain knowledge
- Does not know about LLM providers, tool loops, or checkpoints — those belong to WorkflowContext

## Key Types
- `SchedulerV2` — main scheduler struct
- `TickResult` — `Executed` | `Idle` | `ShutdownRequested`
- `ExecutionMode` — `Sequential` (default) | `Parallel` (tokio tasks)
- `ShutdownSignal` — watch-channel receiver; call `.is_shutdown_requested()`
- `ShutdownTrigger` — sender half; call `.shutdown()`
- `shutdown_signal()` — convenience constructor wiring OS signals

## Design Notes
- `ExecutionMode::Parallel` dispatches workflows as concurrent tokio tasks within a tick; `Sequential` runs them one at a time
- The scheduler does not retry failed ticks — callers drive the loop (typically a `loop { scheduler.tick().await }`)

## Design History
_No changes yet._
