# Concept: Workflow

**Source**: `crates/workflow_engine/src/workflow.rs`

## Responsibility
Defines the contract for a unit of business logic. A workflow receives a `WorkflowContext` each tick and returns a `WorkflowResult` that tells the scheduler what to do next.

## Boundary
- A workflow is **stateless between ticks** — it must not hold mutable cross-tick state in memory. All persistence goes through `WorkflowContext`
- Workflows do not call the scheduler or store directly
- Cross-workflow coordination happens via child spawning (`WorkflowContext::spawn_child`), not direct calls

## Key Types
- `BaseWorkflow` (trait) — `async fn execute(&mut self, ctx: &mut WorkflowContext) -> Result<WorkflowResult>`
- `WorkflowResult` — `Continue` | `Waiting(Vec<PollPredicate>)` | `Finished(type, desc)`
- `PollPredicate` — `{ resource_type, resource_id, step_name, intent_desc }` — what to poll for resumption
- `RouteListener` — declares intent routing for incoming events: `{ intent_desc, msg_filter, needs_llm }`
- `TimeoutConfig` — `{ interval_seconds, max_strikes }` — controls wait_for_event timeout behaviour

## Design Notes
- `WorkflowResult::Waiting` transitions the case to `ExecutionState::Waiting`; the scheduler skips it until a poll match is found
- `WorkflowResult::Finished` triggers `WorkflowContext::finish()` which persists the case and cleans up state

## Design History
_No changes yet._
