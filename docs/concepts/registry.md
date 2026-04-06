# Concept: Registry

**Source**: `crates/workflow_engine/src/registry.rs`

## Responsibility
Maps `workflow_code` strings to factory functions that instantiate `BaseWorkflow` objects. The scheduler uses this to create a workflow handler for each dispatched case.

## Boundary
- Populated once at startup in `register_workflows()` — must not be mutated at runtime
- Has no dependency on stores, LLM providers, or any runtime state
- The registry holds factories (closures), not instances — a fresh workflow object is created per dispatch

## Key API
- `register(code, factory_fn)` — `factory_fn: Fn(Case) -> Box<dyn BaseWorkflow> + Send + Sync + 'static`
- `get(code, case)` — look up and invoke factory; returns `None` for unknown codes
- `registered_codes()` — sorted list of all registered codes (useful for logging/diagnostics)

## Design Notes
- Unknown `workflow_code` values at dispatch time are a misconfiguration, not a runtime error — the scheduler should log and skip, not panic
- Factories receive the `Case` by value so the workflow can own it from the start

## Design History
_No changes yet._
