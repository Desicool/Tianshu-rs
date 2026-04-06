# Concept: Stage

**Source**: `crates/workflow_engine/src/stage.rs`

## Responsibility
Type-safe sub-step within a workflow. Each stage is an async handler that produces a `StageOutcome`, which either advances the workflow to another stage, puts it in a waiting state, or finishes it.

## Boundary
- Stages model one logical step; they must not call other stages' logic directly
- Cross-stage coordination uses checkpoints (`WorkflowContext::get_checkpoint` / `save_checkpoint`), not direct method calls
- Stage enum variants are user-defined per workflow — the engine enforces type safety via the `StageKey` marker trait

## Key Types
- `StageKey` (marker trait) — implement on your stage enum: `Copy + Eq + Hash + Debug + Send + Sync + 'static` + `fn as_str() -> &'static str`
- `StageBase<S>` (trait) — `fn stage_key(&self) -> S` + `async fn execute(&self, ctx: &mut WorkflowContext) -> Result<StageOutcome<S>>`
- `StageOutcome<S>` — `Waiting(Vec<PollPredicate>)` | `Next { stage: S, clear_stages: Vec<S> }` | `Finish { finish_type, finish_desc }`
- Constructor helpers: `StageOutcome::next(stage)`, `StageOutcome::next_with_clear(stage, clear)`, `StageOutcome::finish(t, d)`, `StageOutcome::waiting(polls)`

## Design Notes
- `clear_stages` on `Next` lets a stage erase prior checkpoints to allow replaying earlier steps on a subsequent pass
- The `StageKey` bound being on a user-defined enum means invalid stage transitions are a compile error, not a runtime panic

## Design History
_No changes yet._
