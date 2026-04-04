# Contributing

## Building

```bash
cargo build --workspace
cargo test --workspace
```

## Adding a storage adapter

1. Create a new crate: `crates/workflow_engine_<backend>/`
2. Add `workflow_engine` as a dependency
3. Implement `CaseStore` and/or `StateStore` from `workflow_engine::store`
4. Add an optional `setup()` method that creates tables/collections
5. Add integration tests gated with `#[ignore]`

Example skeleton:

```rust
use async_trait::async_trait;
use anyhow::Result;
use workflow_engine::{case::Case, store::{CaseStore, StateStore, StateEntry}};

pub struct MySqlCaseStore { /* connection pool */ }

#[async_trait]
impl CaseStore for MySqlCaseStore {
    async fn upsert(&self, case: &Case) -> Result<()> { todo!() }
    async fn get_by_key(&self, case_key: &str) -> Result<Option<Case>> { todo!() }
    async fn get_by_session(&self, session_id: &str) -> Result<Vec<Case>> { todo!() }
}
```

## Adding an LLM adapter

1. Create `crates/workflow_engine_llm_<provider>/`
2. Implement `LlmProvider` from `workflow_engine::llm`

```rust
use async_trait::async_trait;
use anyhow::Result;
use workflow_engine::llm::{LlmProvider, LlmRequest, LlmResponse};

pub struct MyLlmProvider;

#[async_trait]
impl LlmProvider for MyLlmProvider {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        todo!()
    }
}
```

## Code style

- `cargo fmt --all`
- `cargo clippy --workspace -- -D warnings`
