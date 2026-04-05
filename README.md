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
| **Tool orchestration** | ToolNode / custom | `Tool` trait + `ToolRegistry` — read/write concurrency |
| **Streaming LLM** | Via LangChain streaming | First-class `StreamingLlmProvider` trait |
| **Error recovery** | Custom retry logic | `RetryPolicy` + `ResilientLlmProvider` with fallbacks |
| **Sub-workflow spawning** | Subgraphs | `ctx.spawn_child()` — checkpoint-safe, zero resources while waiting |
| **Context management** | Custom | `ManagedConversation` — auto-compacts at configurable threshold |
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

Three small traits. Implement them for your backend of choice:

```rust
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn upsert(&self, session: &Session) -> Result<()>;
    async fn get(&self, session_id: &str) -> Result<Option<Session>>;
    async fn delete(&self, session_id: &str) -> Result<()>;
}

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
    // Session-scoped (cross-case) state methods also available
    async fn save_session(&self, session_id: &str, step: &str, data: &str) -> Result<()>;
    async fn get_session(&self, session_id: &str, step: &str) -> Result<Option<SessionStateEntry>>;
    // ...
}
```

`SessionStore` is intentionally minimal — session structure is highly business-specific, so users should implement it to match their schema. The engine provides `InMemorySessionStore` and `PostgresSessionStore` as reference implementations.

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

### 6. Tool orchestration with concurrency-safe execution

Define tools with the `Tool` trait and register them in a `ToolRegistry`. Call `ctx.tool_step()` to run a full LLM-driven tool-use loop — the engine calls the model, executes tool calls, and feeds results back until the model returns plain text:

```rust
struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str { "web_search" }
    fn description(&self) -> &str { "Search the web for information" }
    fn safety(&self) -> ToolSafety { ToolSafety::ReadOnly }  // runs concurrently with other ReadOnly tools
    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({ "type": "object", "properties": { "query": { "type": "string" } } })
    }
    async fn execute(&self, input: JsonValue) -> Result<String> {
        web_search(input["query"].as_str().unwrap_or("")).await
    }
}

let mut tools = ToolRegistry::new();
tools.register(SearchTool);

let result = ctx.tool_step("research", &llm, &tools, request, &ToolLoopConfig::default()).await?;
```

`ReadOnly` tools run in parallel; `Exclusive` tools run alone. The engine partitions each round of tool calls automatically.

### 7. Error recovery and resilient providers

Wrap any `LlmProvider` with retry and fallback logic:

```rust
let policy = RetryPolicy {
    max_attempts: 3,
    base_delay: Duration::from_secs(1),
    backoff_factor: 2.0,
    ..Default::default()
};

let llm = ResilientLlmProvider::new(
    Arc::new(OpenAiProvider::new("key", "gpt-4o")),
    policy,
).with_fallback(Arc::new(OpenAiProvider::new("key", "gpt-3.5-turbo")));
```

Errors are classified automatically: `Transient` retries with backoff, `ProviderOverloaded` tries a fallback provider, `Fatal` stops immediately. Use `ctx.step_with_retry()` to apply retry logic at the step level:

```rust
let result = ctx.step_with_retry("call_llm", &policy, |_| async {
    llm.complete(request.clone()).await
}).await?;
```

### 8. Streaming LLM responses

Implement `StreamingLlmProvider` to deliver `LlmStreamEvent` values as they arrive. The `OpenAiProvider` already implements it out of the box:

```rust
let (tx, mut rx) = tokio::sync::mpsc::channel(64);
llm.stream(request, tx).await?;

while let Some(event) = rx.recv().await {
    match event {
        LlmStreamEvent::TextDelta(s) => print!("{}", s),
        LlmStreamEvent::ToolUse(call) => handle_tool(call).await?,
        LlmStreamEvent::Done(_) => break,
        LlmStreamEvent::Error(e) => return Err(anyhow::anyhow!(e)),
        _ => {}
    }
}
```

### 9. Sub-workflow spawning

A workflow can spawn child workflows and wait for them — with full checkpoint safety. The parent suspends as `Waiting`, freeing all threads and connections until all children finish:

```rust
let handles = ctx.spawn_children(
    &case_store,
    vec![
        SpawnConfig { workflow_code: "analyze".into(), resource_data: Some(chunk_a), ..Default::default() },
        SpawnConfig { workflow_code: "analyze".into(), resource_data: Some(chunk_b), ..Default::default() },
    ],
).await?;

// Parent suspends here — no thread held — resumes when all children are done
let result = ctx.await_children(&handles, &case_store).await?;
```

### 10. Token-aware context compaction

`ManagedConversation` tracks token usage and automatically compacts conversation history before hitting context limits:

```rust
let mut conv = ManagedConversation::new(
    ContextConfig::default(),           // 128k input tokens, compact at 85% threshold
    Arc::new(CharTokenCounter),
    TruncationCompaction { preserve_recent: 10 },
);

conv.push(user_msg).await?;           // auto-compacts if approaching the limit
ctx.set_managed_conversation(conv);   // attach to workflow context
```

Use `LlmSummaryCompaction` to summarise dropped messages with an LLM call instead of truncating them.

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
    session::Session,
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

    let session = Session::new("session_1");
    let case = Case::new("case_1".into(), "session_1".into(), "hello".into());
    let mut env = SchedulerEnvironment::new(session, vec![case]);
    let mut scheduler = SchedulerV2::new();

    scheduler.tick(&mut env, &registry, cs, ss, None, None).await?;
    Ok(())
}
```

See [`examples/approval_workflow`](examples/approval_workflow/) for a complete example with polling and stage transitions.

---

## Sessions and cross-case variables

A **session** groups related cases. One session can have multiple cases running in parallel:

```rust
use workflow_engine::{session::Session, engine::{SchedulerEnvironment, ExecutionMode}};

let session = Session::new("session_1")
    .with_metadata(serde_json::json!({"user": "alice"}));

let cases = vec![case_a, case_b, case_c]; // all share session_1
let env = SchedulerEnvironment::new(session, cases)
    .with_execution_mode(ExecutionMode::Parallel);
```

Cases within a session can share **session-scoped variables** — state that is visible across cases:

```rust
// In any workflow's run() method:
ctx.set_session_state("shared_counter", 42).await?;

// Another case in the same session can read it:
let val: i32 = ctx.get_session_state("shared_counter", 0).await?;
```

Session-scoped variables use last-write-wins semantics. **The engine provides no locking** — workflows that need concurrency control for shared variables must implement it themselves.

---

## Crates

| Crate | Description |
|---|---|
| `workflow_engine` | Core library: scheduler, traits, in-memory stores |
| `workflow_engine_postgres` | PostgreSQL adapters for `SessionStore` + `CaseStore` + `StateStore` |
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

**Recently implemented:**
- [x] Tool orchestration — `Tool` trait, `ToolRegistry`, `ctx.tool_step()`
- [x] Error recovery — `RetryPolicy`, `ResilientLlmProvider`, `ctx.step_with_retry()`
- [x] Streaming LLM — `StreamingLlmProvider`, `LlmStreamEvent`, OpenAI SSE streaming
- [x] Sub-workflow spawning — `ctx.spawn_child()`, `ctx.await_children()`
- [x] Context management — `ManagedConversation`, `TruncationCompaction`, `LlmSummaryCompaction`

**Up next:**
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
