# Concept: Case

**Source**: `crates/workflow_engine/src/case.rs`

## Responsibility
The persistent record of a single workflow execution instance. Carries identity, lifecycle state, resource payload, and parent/child relationships.

## Boundary
- `Case` is a **pure data struct** — no methods that trigger side effects
- All mutations are committed through `CaseStore::upsert`
- The engine creates cases; user code reads `case.resource_data` and `case.execution_state`

## Key Fields
- `case_key: String` — unique identifier
- `session_id: String` — which session this belongs to
- `workflow_code: String` — which workflow handles this case (looked up in WorkflowRegistry)
- `execution_state: ExecutionState` — `Running` | `Waiting` | `Finished`
- `resource_data: Option<JsonValue>` — the business payload; workflows read and update this
- `parent_key: Option<String>` / `child_keys: Vec<String>` — child workflow relationships
- `finished_type: Option<String>` / `finished_description: Option<String>` — set by `WorkflowContext::finish()`
- `lifecycle_state: String` — user-defined lifecycle tag (e.g. "active", "archived")
- `private_vars: Option<JsonValue>` — internal workflow variables not exposed to external consumers
- `processing_report: Vec<JsonValue>` — append-only audit log entries

## Design Notes
- `ExecutionState` drives the scheduler: only `Running` cases are dispatched each tick; `Waiting` cases are skipped until a poll match resumes them; `Finished` cases are never dispatched again

## Design History
_No changes yet._
