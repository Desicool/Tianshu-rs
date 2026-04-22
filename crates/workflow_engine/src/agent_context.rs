// Copyright 2026 Desicool
// SPDX-License-Identifier: Apache-2.0

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use uuid::Uuid;

use crate::agent::{Agent, AgentId, Capabilities, CapabilityRestriction};
use crate::case::Case;
use crate::compact::{CompactionStrategy, ManagedConversation, TruncationCompaction};
use crate::context::WorkflowContext;
use crate::llm::{LlmMessage, LlmProvider, LlmRequest};
use crate::observe::{AgentCompleteRecord, AgentMessageRecord, AgentSpawnRecord};
use crate::retry::RetryPolicy;
use crate::spawn::{ChildHandle, ChildStatus, ChildrenResult, SpawnConfig};
use crate::store::{AgentMessage, AgentMessageStore, AgentStore};
use crate::token::{CharTokenCounter, ContextConfig, TokenCounter};
use crate::tool::ToolRegistry;
use crate::tool_loop::{ToolLoopConfig, ToolLoopResult};

/// A persistent conversation that survives across ticks.
///
/// Stored in the agent's case state under key `"wf_agent_conv"`.
/// The dirty flag prevents unnecessary saves.
pub struct PersistentConversation {
    managed: ManagedConversation,
    dirty: bool,
}

impl PersistentConversation {
    const STATE_KEY: &'static str = "wf_agent_conv";

    /// Load from StateStore. Returns an empty conversation if no prior state exists.
    pub async fn load(
        wf_ctx: &mut WorkflowContext,
        context_config: ContextConfig,
        compaction: Arc<dyn CompactionStrategy>,
        counter: Arc<dyn TokenCounter>,
    ) -> Result<Self> {
        let messages: Vec<LlmMessage> = wf_ctx.get_state(Self::STATE_KEY, vec![]).await?;

        let mut managed = ManagedConversation::new(context_config, counter, compaction);
        for msg in messages {
            managed.push(msg);
        }

        Ok(Self {
            managed,
            dirty: false,
        })
    }

    /// Save to StateStore if dirty.
    pub async fn save(&mut self, wf_ctx: &mut WorkflowContext) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let messages = self.managed.messages().to_vec();
        wf_ctx.set_state(Self::STATE_KEY, messages).await?;
        self.dirty = false;
        Ok(())
    }

    /// Save immediately, regardless of dirty flag.
    pub async fn force_save(&mut self, wf_ctx: &mut WorkflowContext) -> Result<()> {
        let messages = self.managed.messages().to_vec();
        wf_ctx.set_state(Self::STATE_KEY, messages).await?;
        self.dirty = false;
        Ok(())
    }

    pub fn managed(&mut self) -> &mut ManagedConversation {
        &mut self.managed
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// Configuration for spawning a child agent.
pub struct AgentSpawnConfig {
    /// The agent ID for the new child (becomes the backing Case's case_key).
    pub agent_id: String,
    /// The role/human-readable name for the child agent.
    pub role: String,
    /// The workflow_code to register the child case under (must already be in WorkflowRegistry).
    pub workflow_code: String,
    /// Optional capability restrictions to apply (narrows parent's capabilities).
    pub restriction: Option<CapabilityRestriction>,
    /// Optional agent-specific config/instructions.
    pub config: Option<JsonValue>,
    /// Optional initial resource_data for the backing Case.
    pub resource_data: Option<JsonValue>,
}

/// A handle to a spawned child agent (wraps ChildHandle from spawn.rs).
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub child_handle: ChildHandle,
}

/// Agent execution context. Borrows `WorkflowContext` for the duration of one tick.
///
/// Capability enforcement is at this boundary — `wf_ctx` is never exposed publicly.
pub struct AgentContext<'a> {
    wf_ctx: &'a mut WorkflowContext,
    agent: Agent,
    tool_registry: Arc<ToolRegistry>,
    message_store: Arc<dyn AgentMessageStore>,
    agent_store: Arc<dyn AgentStore>,
    /// Lazy-loaded persistent conversation; `None` until first `converse()` call.
    conversation: Option<PersistentConversation>,
}

impl<'a> AgentContext<'a> {
    pub fn new(
        wf_ctx: &'a mut WorkflowContext,
        agent: Agent,
        tool_registry: Arc<ToolRegistry>,
        message_store: Arc<dyn AgentMessageStore>,
        agent_store: Arc<dyn AgentStore>,
    ) -> Self {
        Self {
            wf_ctx,
            agent,
            tool_registry,
            message_store,
            agent_store,
            conversation: None,
        }
    }

    // ── Identity ──────────────────────────────────────────────────────────────

    pub fn agent_id(&self) -> &AgentId {
        &self.agent.agent_id
    }

    pub fn role(&self) -> &str {
        &self.agent.role
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.agent.capabilities
    }

    pub fn case(&self) -> &Case {
        &self.wf_ctx.case
    }

    // ── Checkpoint / step ─────────────────────────────────────────────────────

    pub async fn step<F, Fut, T>(&mut self, name: &str, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<T>> + Send,
        T: serde::Serialize + serde::de::DeserializeOwned + Send,
    {
        // WorkflowContext::step passes &mut Self to the closure; we drop that arg.
        self.wf_ctx.step(name, move |_ctx| f()).await
    }

    pub async fn step_with_retry<F, Fut, T>(
        &mut self,
        name: &str,
        policy: &RetryPolicy,
        f: F,
    ) -> Result<T>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<T>> + Send,
        T: serde::Serialize + serde::de::DeserializeOwned + Send,
    {
        self.wf_ctx.step_with_retry(name, policy, f).await
    }

    // ── Case-scoped state ─────────────────────────────────────────────────────

    pub async fn get_state<T>(&mut self, name: &str, default: T) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send,
    {
        self.wf_ctx.get_state(name, default).await
    }

    pub async fn set_state<T>(&mut self, name: &str, value: T) -> Result<()>
    where
        T: serde::Serialize + Send,
    {
        self.wf_ctx.set_state(name, value).await
    }

    // ── Session-scoped state (capability-checked) ─────────────────────────────

    pub async fn get_session_state<T>(&mut self, name: &str, default: T) -> Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send,
    {
        if !self.agent.capabilities.is_session_key_visible(name) {
            return Err(anyhow!(
                "agent '{}' does not have visibility to session key '{}'",
                self.agent.agent_id,
                name
            ));
        }
        self.wf_ctx.get_session_state(name, default).await
    }

    pub async fn set_session_state<T>(&mut self, name: &str, value: T) -> Result<()>
    where
        T: serde::Serialize + Send,
    {
        if !self.agent.capabilities.is_session_key_writable(name) {
            return Err(anyhow!(
                "agent '{}' does not have write access to session key '{}'",
                self.agent.agent_id,
                name
            ));
        }
        self.wf_ctx.set_session_state(name, value).await
    }

    // ── Tool access ───────────────────────────────────────────────────────────

    pub fn scoped_tools(&self) -> crate::tool::ScopedToolRegistry<'_> {
        crate::tool::ScopedToolRegistry::new(&self.tool_registry, &self.agent.capabilities)
    }

    /// Run an isolated tool loop (single-shot — conversation discarded after).
    /// Checkpoints the final_text so the step is never re-executed on restart.
    ///
    /// **Replay note**: on a warm restart where the checkpoint already exists,
    /// only `final_text` is restored from storage. The returned `ToolLoopResult`
    /// will have `messages = []`, `rounds = 0`, and `total_tool_calls = 0` — they
    /// are not persisted and are not meaningful after a restart.
    pub async fn tool_step(
        &mut self,
        name: &str,
        llm: &dyn LlmProvider,
        request: LlmRequest,
        config: &ToolLoopConfig,
    ) -> Result<ToolLoopResult> {
        // Check for an existing checkpoint — if found, reconstruct a ToolLoopResult from it.
        if let Some(cached) = self.wf_ctx.get_checkpoint(name).await? {
            if !cached.is_null() {
                let final_text: String = serde_json::from_value(cached)?;
                return Ok(ToolLoopResult {
                    final_text,
                    messages: vec![],
                    rounds: 0,
                    total_tool_calls: 0,
                });
            }
        }

        let scoped =
            crate::tool::ScopedToolRegistry::new(&self.tool_registry, &self.agent.capabilities);
        let agent_id = self.agent.agent_id.clone();
        let observer = self.wf_ctx.observer();

        let result = crate::tool_loop::run_agent_tool_loop(
            llm, request, &scoped, config, observer, &agent_id,
        )
        .await?;

        // Save checkpoint so the step is idempotent on restart.
        self.wf_ctx
            .save_checkpoint(name, serde_json::to_value(&result.final_text)?)
            .await?;

        Ok(result)
    }

    // ── Agent spawning ────────────────────────────────────────────────────────

    pub async fn spawn_agent(&mut self, config: AgentSpawnConfig) -> Result<AgentHandle> {
        if !self.agent.capabilities.can_spawn {
            return Err(anyhow!(
                "agent '{}' does not have spawn capability",
                self.agent.agent_id
            ));
        }
        if self.agent.capabilities.max_spawn_depth == 0 {
            return Err(anyhow!(
                "agent '{}' has reached max spawn depth",
                self.agent.agent_id
            ));
        }

        let restriction = config.restriction.unwrap_or(CapabilityRestriction {
            allowed_tools: None,
            can_spawn: None,
            max_spawn_depth: None,
            can_send_messages: None,
            visible_session_keys: None,
            writable_session_keys: None,
        });
        let child_capabilities = self.agent.capabilities.narrow(&restriction);

        let now = Utc::now();
        let child_agent = Agent {
            agent_id: AgentId::new(config.agent_id.clone()),
            role: config.role.clone(),
            capabilities: child_capabilities,
            parent_agent_id: Some(self.agent.agent_id.clone()),
            child_agent_ids: vec![],
            config: config.config,
            created_at: now,
            updated_at: now,
        };

        // Persist the child agent record
        self.agent_store.upsert(&child_agent).await?;

        // Register child in parent's child list
        self.agent
            .child_agent_ids
            .push(child_agent.agent_id.clone());
        self.agent_store.upsert(&self.agent).await?;

        // Create the backing Case via WorkflowContext::spawn_child
        let spawn_cfg = SpawnConfig {
            workflow_code: config.workflow_code,
            resource_data: config.resource_data,
            case_key: Some(config.agent_id),
        };
        let child_handle = self.wf_ctx.spawn_child(spawn_cfg).await?;

        // Emit observer event
        if let Some(obs) = self.wf_ctx.observer() {
            obs.on_agent_spawn(&AgentSpawnRecord {
                parent_agent_id: self.agent.agent_id.clone(),
                child_agent_id: child_agent.agent_id.clone(),
                child_role: child_agent.role.clone(),
                timestamp: Utc::now(),
            })
            .await;
        }

        Ok(AgentHandle {
            agent_id: child_agent.agent_id,
            child_handle,
        })
    }

    // ── Agent communication ───────────────────────────────────────────────────

    pub async fn send_message(&mut self, to: &AgentId, payload: JsonValue) -> Result<()> {
        if !self.agent.capabilities.can_send_messages {
            return Err(anyhow!(
                "agent '{}' does not have messaging capability",
                self.agent.agent_id
            ));
        }

        let message_id = Uuid::new_v4().to_string();
        let message = AgentMessage {
            message_id: message_id.clone(),
            from_agent_id: self.agent.agent_id.clone(),
            to_agent_id: to.clone(),
            payload,
            created_at: Utc::now(),
            consumed: false,
        };

        self.message_store.send(&message).await?;

        if let Some(obs) = self.wf_ctx.observer() {
            obs.on_agent_message(&AgentMessageRecord {
                message_id,
                from_agent_id: self.agent.agent_id.clone(),
                to_agent_id: to.clone(),
                timestamp: Utc::now(),
            })
            .await;
        }

        Ok(())
    }

    pub async fn receive_messages(&mut self) -> Result<Vec<AgentMessage>> {
        self.message_store.receive(&self.agent.agent_id).await
    }

    pub async fn acknowledge_messages(&mut self, message_ids: &[String]) -> Result<()> {
        self.message_store.acknowledge(message_ids).await
    }

    // ── Child status ──────────────────────────────────────────────────────────

    pub async fn child_status(&self, handle: &AgentHandle) -> Result<ChildStatus> {
        self.wf_ctx.child_status(&handle.child_handle).await
    }

    pub async fn await_children(&self, handles: &[AgentHandle]) -> Result<ChildrenResult> {
        let child_handles: Vec<ChildHandle> =
            handles.iter().map(|h| h.child_handle.clone()).collect();
        self.wf_ctx.await_children(&child_handles).await
    }

    // ── Conversation management ───────────────────────────────────────────────

    pub fn set_managed_conversation(&mut self, conv: ManagedConversation) {
        self.wf_ctx.set_managed_conversation(conv);
    }

    pub fn managed_conversation(&mut self) -> Option<&mut ManagedConversation> {
        self.wf_ctx.managed_conversation()
    }

    // ── Persistent conversation ───────────────────────────────────────────────

    /// Run a persistent conversation turn.
    ///
    /// Loads prior conversation history on first call, appends the user message,
    /// compacts if needed (with immediate save on compaction), calls the LLM with
    /// the agent's scoped tools, and appends all new messages from the tool loop.
    ///
    /// The conversation is saved at tick boundary by `AgentWorkflowInstance::run()`.
    /// Pass `model` explicitly since `LlmRequest` requires it.
    pub async fn converse(
        &mut self,
        user_content: &str,
        llm: &dyn LlmProvider,
        model: &str,
        system_prompt: Option<&str>,
        config: &ToolLoopConfig,
    ) -> Result<String> {
        // 1. Lazy-load conversation
        if self.conversation.is_none() {
            let conv = PersistentConversation::load(
                self.wf_ctx,
                ContextConfig::default(),
                Arc::new(TruncationCompaction { preserve_recent: 4 }),
                Arc::new(CharTokenCounter),
            )
            .await?;
            self.conversation = Some(conv);
        }

        // 2. Append user message and compact if needed.
        //    We do this in a block to release the borrow on self.conversation
        //    before we need to borrow self.wf_ctx and self.tool_registry.
        let (messages, old_len, compacted) = {
            let conv = self.conversation.as_mut().unwrap();
            conv.managed().push(LlmMessage {
                role: "user".to_string(),
                content: user_content.to_string(),
                tool_calls: None,
                tool_call_id: None,
            });
            conv.mark_dirty();

            // 3. Compact if needed — save immediately since compaction is lossy.
            let compacted = conv.managed().compact_if_needed().await?;

            let messages = conv.managed().messages().to_vec();
            let old_len = messages.len();
            (messages, old_len, compacted)
        };

        // If compaction fired, force-save now so the compacted state is durable
        // before the LLM call. We release the conv borrow first (it ended with the block above)
        // so we can mutably borrow self.wf_ctx.
        if compacted {
            let conv = self.conversation.as_mut().unwrap();
            conv.force_save(self.wf_ctx).await?;
        }

        // 4. Build tool-aware LLM request from current conversation history.
        let scoped =
            crate::tool::ScopedToolRegistry::new(&self.tool_registry, &self.agent.capabilities);
        let agent_id = self.agent.agent_id.clone();
        let observer = self.wf_ctx.observer();

        let request = LlmRequest {
            model: model.to_string(),
            system_prompt: system_prompt.map(|s| s.to_string()),
            messages,
            temperature: None,
            max_tokens: None,
            tools: None, // tool loop sets this internally
        };

        // 5. Run tool loop.
        let result = crate::tool_loop::run_agent_tool_loop(
            llm, request, &scoped, config, observer, &agent_id,
        )
        .await?;

        // 6. Append new messages (from index old_len onward) to the persistent conversation.
        let conv = self.conversation.as_mut().unwrap();
        for msg in result.messages.iter().skip(old_len) {
            conv.managed().push(msg.clone());
        }
        conv.mark_dirty();

        Ok(result.final_text)
    }

    /// Save conversation to StateStore if dirty (called at tick boundary).
    pub async fn save_conversation_if_dirty(&mut self) -> Result<()> {
        if let Some(conv) = &mut self.conversation {
            conv.save(self.wf_ctx).await?;
        }
        Ok(())
    }

    // ── Completion ────────────────────────────────────────────────────────────

    pub async fn finish(&mut self, finished_type: String, desc: String) -> Result<()> {
        if let Some(obs) = self.wf_ctx.observer() {
            obs.on_agent_complete(&AgentCompleteRecord {
                agent_id: self.agent.agent_id.clone(),
                role: self.agent.role.clone(),
                finished_type: finished_type.clone(),
                finished_description: desc.clone(),
                timestamp: Utc::now(),
            })
            .await;
        }
        self.wf_ctx.finish(finished_type, desc).await
    }
}
