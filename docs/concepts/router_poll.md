# Concept: Router / Poll

**Source**: `crates/workflow_engine/src/poll.rs`

## Responsibility
Two related concerns:

1. **PollEvaluator** — given a list of `PollPredicate`s and a `ResourceFetcher`, returns which predicates have been satisfied (i.e. their resource now exists). Used by the scheduler to determine which Waiting cases can resume.

2. **IntentRouterV2** — given an incoming event and a set of candidate workflows (each with a `RouteListener`), uses an LLM to classify the event and route it to the correct workflow.

## Boundary
- The router and evaluator are **stateless between evaluations** — they hold no per-case state
- Routing decisions are not persisted by the router; the caller (scheduler or integration layer) is responsible for acting on the result
- `ResourceFetcher` is a user-implemented trait — it bridges the engine to whatever event source the application uses (database poll, message queue, HTTP endpoint, etc.)

## Key Types
- `ResourceFetcher` (trait) — `async fn fetch(resource_type, resource_id) -> Result<Option<JsonValue>>`
- `PollEvaluator` — `async fn evaluate(polls, fetcher) -> Vec<PollMatch>`
- `PollMatch` — `{ step_name: String, payload: JsonValue }`
- `IntentRouterV2` — LLM-based event classifier

## Design Notes
- `PollEvaluator::evaluate` is O(n) over the predicate list — for high-frequency polling, implementations should batch fetches in their `ResourceFetcher`
- `IntentRouterV2` is optional; simple workflows can use `PollEvaluator` alone with deterministic resource keys

## Design History
_No changes yet._
