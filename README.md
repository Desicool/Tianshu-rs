# Tianshu 天枢

> *The celestial pivot — the simplest way to build long-running AI agent workflow orchestration in Rust.*

[简体中文](README.zh.md) | English

---

## What is Tianshu?

Tianshu is a **checkpoint-safe, coroutine-like workflow engine** for building AI agent orchestration systems in Rust.

Most workflow frameworks ask you to think in graphs: define nodes, connect edges, wire up state schemas. Tianshu takes a different approach — **you write normal sequential async code**. Each `ctx.step()` call is automatically checkpointed. If your process crashes, it resumes from the last completed step with zero extra configuration.

```rust
async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
    // Each step is automatically checkpointed.
    // Crash here? Restart and it picks up from the last completed step.
    let search_results = ctx.step("search_web", |_| async {
        web_search("latest AI papers").await
    }).await?;

    let summary = ctx.step("summarize", |_| async {
        llm_summarize(&search_results).await
    }).await?;

    ctx.finish("success", summary.clone()).await?;
    Ok(WorkflowResult::Finished("success".into(), summary))
}
```

That's it. No node definitions. No edge wiring. No state schema. Just code.

---

## The core idea

The mental model is borrowed from **coroutines**: suspend at a checkpoint, resume later. Except you don't write coroutine boilerplate — `ctx.step()` handles it for you.

| What you write | What happens |
|---|---|
| `ctx.step("name", \|_\| async { ... })` | Step executes; result is persisted to storage |
| Process crashes mid-step | Next run re-executes only that step |
| Process crashes between steps | Next run skips all completed steps, resumes from the failed one |
| Step completes normally | Checkpoint is stored; never re-executed |

This makes **long-term tasks** natural. A workflow can `return Waiting(...)` to sleep for hours or days until an external event arrives — without holding any thread or connection.

---

## Tianshu vs LangGraph

LangGraph is an excellent tool. Tianshu solves a different problem: you want **the simplest possible mental model** for building durable, long-running agent workflows in a production Rust service.

|  | **LangGraph** | **Tianshu** |
|---|---|---|
| **Language** | Python | Rust |
| **Mental model** | Graph: define nodes + edges explicitly | Coroutine-like: write sequential async code |
| **Checkpointing** | Configure a `checkpointer` on graph compile | Automatic — every `ctx.step()` is a checkpoint |
| **Crash recovery** | Resumes from last checkpoint if configured | Always on — restart process, resume from last step |
| **Long-running tasks** | Supported | First-class — `Waiting(polls)` suspends with zero resources |
| **Storage backends** | SQLite, PostgreSQL, Redis (official) | Any database — implement two small traits |
| **LLM integration** | Via LangChain (Python ecosystem) | Via `LlmProvider` trait — any API, any vendor |
| **Concurrency** | Python asyncio | Rust Tokio — native async/await, no GIL |
| **Observability** | LangSmith (commercial platform) | Structured logging via `tracing` (see [Observability](#observability)) |
| **License** | MIT | MIT |

### Code comparison

**LangGraph** — define nodes, connect edges, compile:
```python
from langgraph.graph import StateGraph

def search_node(state: State) -> dict:
    return {"results": web_search(state["query"])}

def summarize_node(state: State) -> dict:
    return {"summary": llm_summarize(state["results"])}

builder = StateGraph(State)
builder.add_node("search", search_node)
builder.add_node("summarize", summarize_node)
builder.add_edge("search", "summarize")
graph = builder.compile(checkpointer=SqliteSaver.from_conn_string(":memory:"))
```

**Tianshu** — just write the flow:
```rust
async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
    let results = ctx.step("search", |_| async { web_search(&query).await }).await?;
    let summary  = ctx.step("summarize", |_| async { llm_summarize(&results).await }).await?;
    ctx.finish("ok", summary.clone()).await?;
    Ok(WorkflowResult::Finished("ok".into(), summary))
}
```

---

## Key advantages

### 1. Coroutine-like — reads like normal code

No graph topology, no state schemas, no node/edge wiring. The flow of your workflow **is** the code. A new engineer can read it top to bottom and understand it.

### 2. Automatic crash recovery

Every `ctx.step()` persists its result before returning. On restart, completed steps are skipped, and execution resumes from exactly where it left off. You get fault tolerance for free, without thinking about it.

### 3. Long-term task support

Poll predicates let a workflow declare *"wake me up when X arrives"* and then suspend cleanly:

```rust
return Ok(WorkflowResult::Waiting(vec![PollPredicate {
    resource_type: "review_decision".into(),
    resource_id: ctx.case.case_key.clone(),
    step_name: "await_decision".into(),
    intent_desc: None,
}]));
```

The workflow holds no thread, no connection, no memory while waiting. It can resume hours or days later.

### 4. Database-agnostic storage

Two small traits. Implement them for your backend of choice:

```rust
#[async_trait]
pub trait CaseStore: Send + Sync {
    async fn upsert(&self, case: &Case) -> Result<()>;
    async fn get_by_key(&self, case_key: &str) -> Result<Option<Case>>;
    async fn get_by_session(&self, session_id: &str) -> Result<Vec<Case>>;
}

#[async_trait]
pub trait StateStore: Send + Sync {
    async fn save(&self, case_key: &str, step: &str, data: &str) -> Result<()>;
    async fn get(&self, case_key: &str, step: &str) -> Result<Option<StateEntry>>;
    async fn get_all(&self, case_key: &str) -> Result<Vec<StateEntry>>;
    async fn delete_by_case(&self, case_key: &str) -> Result<()>;
}
```

The community can (and should) build adapters for MySQL, MongoDB, Redis, DynamoDB, and anything else.

### 5. Any LLM, any vendor

```rust
let llm = OpenAiProvider::new("sk-...", "gpt-4o");

// or Ollama (local)
let llm = OpenAiProvider::builder("ignored", "llama3")
    .base_url("http://localhost:11434/v1")
    .build();

// or Doubao
let llm = OpenAiProvider::builder("your-key", "doubao-seed-2-0-pro-260215")
    .base_url("https://ark.cn-beijing.volces.com/api/v3")
    .build();
```

---

## Quick start

```toml
[dependencies]
workflow_engine = { git = "https://github.com/your-org/tianshu-rs" }
```

```rust
use std::sync::Arc;
use workflow_engine::{
    case::Case,
    context::WorkflowContext,
    engine::{SchedulerEnvironment, SchedulerV2},
    store::{InMemoryCaseStore, InMemoryStateStore},
    workflow::{BaseWorkflow, WorkflowResult},
    WorkflowRegistry,
};

struct HelloWorkflow;

#[async_trait::async_trait]
impl BaseWorkflow for HelloWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> anyhow::Result<WorkflowResult> {
        let msg: String = ctx.step("greet", |ctx| async move {
            Ok(format!("Hello from {}!", ctx.case.case_key))
        }).await?;

        ctx.finish("success".into(), msg.clone()).await?;
        Ok(WorkflowResult::Finished("success".into(), msg))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut registry = WorkflowRegistry::new();
    registry.register("hello", |_| Box::new(HelloWorkflow));

    let cs = Arc::new(InMemoryCaseStore::default());
    let ss = Arc::new(InMemoryStateStore::default());
    let case = Case::new("case_1".into(), "session_1".into(), "hello".into());
    let mut env = SchedulerEnvironment::new("session_1", vec![case]);
    let mut scheduler = SchedulerV2::new();

    scheduler.tick(&mut env, &registry, cs, ss, None).await?;
    Ok(())
}
```

See [`examples/approval_workflow`](examples/approval_workflow/) for a complete example with polling and stage transitions.

---

## Crates

| Crate | Description |
|---|---|
| `workflow_engine` | Core library: scheduler, traits, in-memory stores |
| `workflow_engine_postgres` | PostgreSQL adapters for `CaseStore` + `StateStore` |
| `workflow_engine_llm_openai` | `LlmProvider` adapter for OpenAI-compatible APIs |

---

## Running the example

```bash
# Start PostgreSQL (optional)
docker-compose up -d

# Run with in-memory stores
cargo run -p approval_workflow

# Run with PostgreSQL
DATABASE_URL=postgres://postgres:postgres@localhost/workflow_engine \
  cargo run -p approval_workflow -- --postgres "$DATABASE_URL"
```

---

## Observability

Tianshu emits structured logs via the [`tracing`](https://docs.rs/tracing) crate at every meaningful state transition:

- Scheduler tick phases (partition → probe → evaluate → execute)
- Workflow state changes (Running → Waiting → Finished)
- Poll predicate evaluation and matches
- Checkpoint save and restore
- Database operations and LLM calls

Configure output format via `tracing-subscriber`:

```rust
// JSON (for log aggregators)
tracing_subscriber::fmt().json().with_env_filter("info").init();

// Text (for development)
tracing_subscriber::fmt().with_env_filter("debug").init();
```

**What's not yet implemented:**
- Step-level timing / duration spans
- Prometheus / metrics counters
- OpenTelemetry / distributed tracing

These are good first contributions. See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Roadmap

- [ ] OpenTelemetry integration (trace context propagation)
- [ ] Step-level timing spans
- [ ] `workflow_engine_sqlite` — SQLite adapter for lightweight deployments
- [ ] `workflow_engine_mongodb` — MongoDB adapter
- [ ] `workflow_engine_llm_anthropic` — Claude API adapter
- [ ] Intent routing example (LLM-based message classification)
- [ ] Admin API for inspecting and replaying workflows

---

## Running tests

```bash
# Unit + in-memory integration tests (no database required)
cargo test --workspace

# PostgreSQL integration tests
DATABASE_URL=postgres://postgres:postgres@localhost/workflow_engine \
  cargo test -p workflow_engine_postgres -- --ignored
```
