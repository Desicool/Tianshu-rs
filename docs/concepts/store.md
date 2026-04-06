# Concept: Store

**Source**: `crates/workflow_engine/src/store.rs`

## Responsibility
Persistence abstraction layer. Three independent traits cover different data concerns: case lifecycle, step runtime state, and session management.

## Boundary
- The engine only depends on these traits — it never imports a concrete store type
- Injected at startup; cannot be swapped at runtime
- In-memory implementations (`InMemoryCaseStore`, `InMemoryStateStore`) are provided for tests and simple use cases
- PostgreSQL implementations live in `crates/workflow_engine_postgres`

## Key Traits

### `CaseStore`
- `upsert(case)` — insert or update a case record
- `get_by_key(case_key)` — fetch one case
- `get_by_session(session_id)` — list all cases in a session
- `setup()` — optional; create tables/indexes on first use

### `StateStore`
- `get(case_key, step)` → `Option<StateEntry>` — read a step's persisted data
- `save(case_key, step, data)` — write a step's data
- `get_session(session_id, step)` / `save_session(session_id, step, data)` — session-scoped variants
- `delete_by_case(case_key)` — clean up all state for a finished case

### `SessionStore`
- `upsert(session)` / `get(session_id)` / `delete(session_id)`
- `setup()` — optional

## Key Types
- `StateEntry` — `{ case_key, step, data: String, updated_at }`
- `SessionStateEntry` — `{ session_id, step, data: String, updated_at }`

## Design Notes
- `StateStore::data` is a raw `String` (JSON-serialized by WorkflowContext); the store layer is encoding-agnostic
- `delete_by_case` is called by `WorkflowContext::finish()` — failure is logged but does not fail the workflow

## Design History
_No changes yet._
