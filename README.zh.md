# 天枢 Tianshu

> *天之枢纽——用 Rust 构建长时 AI Agent 工作流编排系统的最简方式。*

[English](README.md) | 简体中文

---

## 天枢是什么？

天枢是一个**支持断点续传的协程式工作流引擎**，专为在 Rust 服务中构建 AI Agent 编排系统而设计。

大多数工作流框架要求你以"图"的方式思考问题：定义节点、连接边、描述状态结构。天枢采用了完全不同的思路——**你只需要写普通的顺序异步代码**。每一个 `ctx.step()` 调用都会自动完成断点保存。进程崩溃？重启后从最后一个完成的步骤继续，无需任何额外配置。

```rust
async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
    // 每个步骤都自动保存断点。
    // 在这里崩溃了？重启后会从上一个已完成的步骤继续。
    let search_results = ctx.step("search_web", |_| async {
        web_search("最新 AI 论文").await
    }).await?;

    let summary = ctx.step("summarize", |_| async {
        llm_summarize(&search_results).await
    }).await?;

    ctx.finish("success", summary.clone()).await?;
    Ok(WorkflowResult::Finished("success".into(), summary))
}
```

就是这样。没有节点定义，没有边的连接，没有状态结构体。只有代码本身。

---

## 核心理念

天枢的编程模型借鉴了**协程**的思想：在断点处挂起，稍后恢复。但你不需要编写任何协程样板代码——`ctx.step()` 替你完成了一切。

| 你写的代码 | 实际发生的事情 |
|---|---|
| `ctx.step("name", \|_\| async { ... })` | 步骤执行，结果持久化到存储 |
| 进程在步骤执行中途崩溃 | 下次运行只重新执行该步骤 |
| 进程在步骤之间崩溃 | 下次运行跳过所有已完成步骤，从失败处继续 |
| 步骤正常完成 | 断点已保存，永远不会再次执行 |

这让**长时任务**变得自然。工作流可以通过 `return Waiting(...)` 挂起数小时乃至数天，等待外部事件到来——期间不占用任何线程或连接资源。

---

## 天枢 vs LangGraph

LangGraph 是一款优秀的工具。天枢解决的是不同的问题：在生产级 Rust 服务中，你需要**最简单的编程模型**来构建持久化、长时运行的 Agent 工作流。

|  | **LangGraph** | **天枢** |
|---|---|---|
| **语言** | Python | Rust |
| **编程模型** | 图模型：显式定义节点和边 | 协程式：编写顺序异步代码 |
| **断点保存** | 在图编译时配置 `checkpointer` | 自动——每个 `ctx.step()` 就是一个断点 |
| **崩溃恢复** | 配置后可从断点恢复 | 始终开启——重启进程即可从上次步骤继续 |
| **长时任务** | 支持 | 原生支持——`Waiting(polls)` 零资源挂起 |
| **存储后端** | SQLite、PostgreSQL、Redis（官方支持） | 任意数据库——实现两个小接口即可 |
| **LLM 集成** | 通过 LangChain（Python 生态） | 通过 `LlmProvider` trait——支持任意 API、任意厂商 |
| **并发模型** | Python asyncio | Rust Tokio——原生 async/await，无 GIL |
| **可观测性** | LangSmith（商业平台） | 通过 `tracing` 的结构化日志（见[可观测性](#可观测性)） |
| **工具编排** | ToolNode / 自定义 | `Tool` trait + `ToolRegistry`——读写并发安全 |
| **流式 LLM** | 通过 LangChain 流式 | 原生 `StreamingLlmProvider` trait |
| **错误恢复** | 自定义重试逻辑 | `RetryPolicy` + 支持 fallback 的 `ResilientLlmProvider` |
| **子工作流派生** | Subgraph | `ctx.spawn_child()`——断点安全，挂起期间零资源占用 |
| **上下文管理** | 自定义 | `ManagedConversation`——到达阈值自动压缩 |
| **开源协议** | MIT | Apache-2.0 |

### 代码对比

**LangGraph**——定义节点、连接边、编译图：
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

**天枢**——直接写流程：
```rust
async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult> {
    let results = ctx.step("search",    |_| async { web_search(&query).await }).await?;
    let summary  = ctx.step("summarize", |_| async { llm_summarize(&results).await }).await?;
    ctx.finish("ok", summary.clone()).await?;
    Ok(WorkflowResult::Finished("ok".into(), summary))
}
```

---

## 核心优势

### 1. 协程式原语——像普通代码一样可读

没有图拓扑，没有状态结构体，没有节点/边的接线。工作流的**流程就是代码本身**。新工程师从头读到尾就能理解，无需理解框架概念。

### 2. 自动崩溃恢复

每个 `ctx.step()` 在返回之前都会持久化其结果。进程重启后，已完成的步骤会被跳过，执行从中断处精确恢复。容错能力是免费的，无需任何主动设计。

### 3. 长时任务原生支持

轮询断言（Poll Predicate）让工作流可以声明"当 X 到来时唤醒我"，然后干净地挂起：

```rust
return Ok(WorkflowResult::Waiting(vec![PollPredicate {
    resource_type: "review_decision".into(),
    resource_id: ctx.case.case_key.clone(),
    step_name: "await_decision".into(),
    intent_desc: None,
}]));
```

挂起期间工作流不持有任何线程、连接或内存。它可以在数小时或数天后恢复。

### 4. 存储后端无关

只需实现两个小接口，即可接入任何数据库：

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

社区可以（也应该）为 MySQL、MongoDB、Redis、DynamoDB 等构建适配器。

### 5. 任意 LLM，任意厂商

```rust
let llm = OpenAiProvider::new("sk-...", "gpt-4o");

// 本地 Ollama
let llm = OpenAiProvider::builder("ignored", "llama3")
    .base_url("http://localhost:11434/v1")
    .build();

// 字节跳动豆包
let llm = OpenAiProvider::builder("your-key", "doubao-seed-2-0-pro-260215")
    .base_url("https://ark.cn-beijing.volces.com/api/v3")
    .build();
```

### 6. 并发安全的工具编排

通过 `Tool` trait 定义工具，注册到 `ToolRegistry`，再调用 `ctx.tool_step()` 运行完整的 LLM 工具调用循环——引擎自动调用模型、执行工具、将结果反馈给模型，直到模型返回纯文本为止：

```rust
struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str { "web_search" }
    fn description(&self) -> &str { "在网络上搜索信息" }
    fn safety(&self) -> ToolSafety { ToolSafety::ReadOnly }  // 与其他只读工具并行执行
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

`ReadOnly` 工具并行执行；`Exclusive` 工具串行执行。引擎在每轮工具调用中自动进行分区。

### 7. 错误恢复与弹性 LLM 提供者

为任意 `LlmProvider` 添加重试与 fallback 逻辑：

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

错误会被自动分类：`Transient` 指数退避重试，`ProviderOverloaded` 切换到 fallback 提供者，`Fatal` 立即停止。使用 `ctx.step_with_retry()` 在步骤级别应用重试策略：

```rust
let result = ctx.step_with_retry("call_llm", &policy, |_| async {
    llm.complete(request.clone()).await
}).await?;
```

### 8. 流式 LLM 响应

实现 `StreamingLlmProvider` trait 即可逐 token 接收 `LlmStreamEvent`。`OpenAiProvider` 已内置支持：

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

### 9. 子工作流派生

工作流可以派生子工作流并等待它们完成——全程保持断点安全。父工作流以 `Waiting` 状态挂起，释放所有线程和连接资源，直到所有子任务完成：

```rust
let handles = ctx.spawn_children(
    &case_store,
    vec![
        SpawnConfig { workflow_code: "analyze".into(), resource_data: Some(chunk_a), ..Default::default() },
        SpawnConfig { workflow_code: "analyze".into(), resource_data: Some(chunk_b), ..Default::default() },
    ],
).await?;

// 父工作流在此挂起——不占用线程——子任务全部完成后自动恢复
let result = ctx.await_children(&handles, &case_store).await?;
```

### 10. Token 感知的上下文压缩

`ManagedConversation` 追踪 token 用量，并在接近上下文限制时自动压缩对话历史：

```rust
let mut conv = ManagedConversation::new(
    ContextConfig::default(),           // 128k 输入 token，在 85% 时触发压缩
    Arc::new(CharTokenCounter),
    TruncationCompaction { preserve_recent: 10 },
);

conv.push(user_msg).await?;           // 接近限制时自动压缩
ctx.set_managed_conversation(conv);   // 附加到工作流上下文
```

使用 `LlmSummaryCompaction` 可让 LLM 对被截断的消息生成摘要，而非直接丢弃。

---

## 快速上手

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
tianshu = "0.1"

# 可选扩展：
tianshu-llm-openai = "0.1"   # 兼容 OpenAI 接口的 LLM 适配器
tianshu-observe = "0.1"       # 结构化可观测性与链路记录
tianshu-postgres = "0.1"      # PostgreSQL 存储适配器
```

```rust
use std::sync::Arc;
use tianshu::{
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
            Ok(format!("你好，来自 {}！", ctx.case.case_key))
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

完整示例（含轮询等待和阶段转换）参见 [`examples/approval_workflow`](examples/approval_workflow/)。

---

## Crate 列表

所有 crate 均已发布至 [crates.io](https://crates.io)：

| Crate | crates.io | 说明 |
|---|---|---|
| `tianshu` | [![crates.io](https://img.shields.io/crates/v/tianshu.svg)](https://crates.io/crates/tianshu) | 核心库：调度器、trait 接口、内存存储 |
| `tianshu-postgres` | [![crates.io](https://img.shields.io/crates/v/tianshu-postgres.svg)](https://crates.io/crates/tianshu-postgres) | `CaseStore` + `StateStore` 的 PostgreSQL 实现 |
| `tianshu-llm-openai` | [![crates.io](https://img.shields.io/crates/v/tianshu-llm-openai.svg)](https://crates.io/crates/tianshu-llm-openai) | 兼容 OpenAI 接口的 `LlmProvider` 适配器 |
| `tianshu-observe` | [![crates.io](https://img.shields.io/crates/v/tianshu-observe.svg)](https://crates.io/crates/tianshu-observe) | 结构化可观测性与链路记录 |

---

## 运行示例

```bash
# 启动 PostgreSQL（可选）
docker-compose up -d

# 使用内存存储运行
cargo run -p approval_workflow

# 使用 PostgreSQL 运行
DATABASE_URL=postgres://postgres:postgres@localhost/tianshu \
  cargo run -p approval_workflow -- --postgres "$DATABASE_URL"
```

---

## 可观测性

天枢通过 [`tracing`](https://docs.rs/tracing) crate 在每个关键状态转换处输出结构化日志：

- 调度器 tick 各阶段（分区 → 探测 → 评估 → 执行）
- 工作流状态变更（Running → Waiting → Finished）
- 轮询断言评估与匹配
- 断点保存与恢复
- 数据库操作与 LLM 调用

通过 `tracing-subscriber` 配置输出格式：

```rust
// JSON 格式（适合日志聚合平台）
tracing_subscriber::fmt().json().with_env_filter("info").init();

// 文本格式（适合本地开发）
tracing_subscriber::fmt().with_env_filter("debug").init();
```

**尚未实现的功能：**
- 步骤级耗时 / duration span
- Prometheus / metrics 计数器
- OpenTelemetry / 分布式链路追踪

欢迎贡献！详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

---

## 路线图

**近期已实现：**
- [x] 工具编排——`Tool` trait、`ToolRegistry`、`ctx.tool_step()`
- [x] 错误恢复——`RetryPolicy`、`ResilientLlmProvider`、`ctx.step_with_retry()`
- [x] 流式 LLM——`StreamingLlmProvider`、`LlmStreamEvent`、OpenAI SSE 流式输出
- [x] 子工作流派生——`ctx.spawn_child()`、`ctx.await_children()`
- [x] 上下文管理——`ManagedConversation`、`TruncationCompaction`、`LlmSummaryCompaction`

**待开发：**
- [ ] OpenTelemetry 集成（链路上下文传播）
- [ ] 步骤级耗时 span
- [ ] `tianshu-sqlite`——适用于轻量部署的 SQLite 适配器
- [ ] `tianshu-mongodb`——MongoDB 适配器
- [ ] `tianshu-llm-anthropic`——Claude API 适配器
- [ ] 意图路由示例（基于 LLM 的消息分类）
- [ ] 工作流管理 API（查看与重放）

---

## 运行测试

```bash
# 单元测试 + 内存集成测试（无需数据库）
cargo test --workspace

# PostgreSQL 集成测试
DATABASE_URL=postgres://postgres:postgres@localhost/tianshu \
  cargo test -p tianshu-postgres -- --ignored
```
