# Concept: Session

**Source**: `crates/workflow_engine/src/session.rs`

## Responsibility
Groups related cases under a shared identity. Provides a stable anchor for cross-case metadata (user info, channel, conversation tags, etc.).

## Boundary
- Session structure is intentionally minimal — the engine only requires `session_id`
- The `metadata: Option<JsonValue>` field is entirely user-controlled; the engine never reads it
- Session-scoped state variables (in WorkflowContext) use `session_id` as the key namespace

## Key Fields
- `session_id: String`
- `metadata: Option<JsonValue>` — business-defined; e.g. `{ "user_id": "...", "channel": "..." }`
- `created_at`, `updated_at: DateTime<Utc>`

## Design Notes
- Sessions are managed via `SessionStore` (separate from `CaseStore`)
- The engine does not enforce a 1:N session→case cardinality constraint at the storage level — that is a business concern

## Design History
_No changes yet._
