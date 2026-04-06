# tianshu-rs Concept Index

One-line summary per concept. Read this first. Open individual files only when you need boundary details, key types, or design history for that specific concept.

| Concept | Source | Summary | Detail file |
|---------|--------|---------|-------------|
| Scheduler | `crates/workflow_engine/src/engine.rs` | Drives the tick loop — selects runnable cases, dispatches to workflows, handles shutdown. Contains no business logic. | [scheduler.md](scheduler.md) |
| Workflow | `crates/workflow_engine/src/workflow.rs` | Contract for a unit of business logic. Stateless between ticks; all state persisted via WorkflowContext. | [workflow.md](workflow.md) |
| Stage | `crates/workflow_engine/src/stage.rs` | Type-safe sub-step within a workflow. Returns a StageOutcome that advances to the next stage or finishes. | [stage.md](stage.md) |
| WorkflowContext | `crates/workflow_engine/src/context.rs` | Per-tick execution handle. Checkpoints, state vars, child spawning, LLM tool steps, observability. Created fresh each tick. | [workflow_context.md](workflow_context.md) |
| Case | `crates/workflow_engine/src/case.rs` | Persistent record of one workflow execution instance. Pure data struct; mutations committed through CaseStore. | [case.md](case.md) |
| Session | `crates/workflow_engine/src/session.rs` | Groups related cases under a shared identity. Minimal engine-level structure; metadata is user-controlled JSON. | [session.md](session.md) |
| Store | `crates/workflow_engine/src/store.rs` | Persistence abstraction: CaseStore, StateStore, SessionStore traits. Injected at startup; never imported as concrete types by the engine. | [store.md](store.md) |
| Registry | `crates/workflow_engine/src/registry.rs` | Maps workflow_code strings to factory functions. Populated at startup, never mutated at runtime. | [registry.md](registry.md) |
| Router / Poll | `crates/workflow_engine/src/poll.rs` | Evaluates PollPredicates against a ResourceFetcher to resume waiting workflows. IntentRouterV2 uses LLM for event classification. | [router_poll.md](router_poll.md) |
| LLM Abstraction | `crates/workflow_engine/src/llm.rs`, `llm_resilient.rs` | Provider-agnostic LLM traits. Concrete providers live in separate crates and are injected at startup. | [llm_abstraction.md](llm_abstraction.md) |
| Tool Loop | `crates/workflow_engine/src/tool_loop.rs` | Runs the LLM ↔ tool execution loop until final answer or max_rounds. Invoked only via WorkflowContext::tool_step(). | [tool_loop.md](tool_loop.md) |
| Observability | `crates/workflow_engine_observe/` | Collects step, LLM call, and workflow records. Side-effect-only — never influences execution. | [observability.md](observability.md) |
