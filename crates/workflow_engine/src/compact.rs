// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::llm::{LlmMessage, LlmProvider, LlmRequest};
use crate::token::{ContextConfig, TokenCounter};

/// Strategy for compacting a conversation when it approaches the context limit.
#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    async fn compact(
        &self,
        messages: &[LlmMessage],
        target_tokens: u32,
        counter: &dyn TokenCounter,
    ) -> Result<Vec<LlmMessage>>;
}

/// Drop oldest messages until under target, always preserving `preserve_recent` newest.
pub struct TruncationCompaction {
    pub preserve_recent: usize,
}

#[async_trait]
impl CompactionStrategy for TruncationCompaction {
    async fn compact(
        &self,
        messages: &[LlmMessage],
        target_tokens: u32,
        counter: &dyn TokenCounter,
    ) -> Result<Vec<LlmMessage>> {
        let len = messages.len();
        let preserve = self.preserve_recent.min(len);
        let suffix_start = len - preserve;

        // Always keep the last `preserve_recent` messages
        let suffix = &messages[suffix_start..];
        let suffix_tokens = counter.count_messages(suffix);

        if suffix_tokens >= target_tokens {
            // Even the preserved messages exceed target; return them anyway
            return Ok(suffix.to_vec());
        }

        // Walk backwards through the prefix, adding messages while under budget
        let mut budget = target_tokens - suffix_tokens;
        let mut keep_from = suffix_start;
        for i in (0..suffix_start).rev() {
            let msg_tokens = counter.count_text(&messages[i].content);
            if msg_tokens > budget {
                break;
            }
            budget -= msg_tokens;
            keep_from = i;
        }

        Ok(messages[keep_from..].to_vec())
    }
}

/// Ask an LLM to summarize the older conversation prefix, then append recent messages verbatim.
pub struct LlmSummaryCompaction {
    pub llm: Arc<dyn LlmProvider>,
    pub model: String,
    pub preserve_recent: usize,
}

#[async_trait]
impl CompactionStrategy for LlmSummaryCompaction {
    async fn compact(
        &self,
        messages: &[LlmMessage],
        _target_tokens: u32,
        _counter: &dyn TokenCounter,
    ) -> Result<Vec<LlmMessage>> {
        let len = messages.len();
        let preserve = self.preserve_recent.min(len);
        let split = len - preserve;

        if split == 0 {
            return Ok(messages.to_vec());
        }

        let prefix = &messages[..split];
        let suffix = &messages[split..];

        // Build a summarization request
        let mut summary_messages = Vec::with_capacity(prefix.len() + 1);
        summary_messages.extend_from_slice(prefix);
        summary_messages.push(LlmMessage {
            role: "user".into(),
            content: "Please provide a concise summary of the conversation above.".into(),
            tool_calls: None,
            tool_call_id: None,
        });

        let request = LlmRequest {
            model: self.model.clone(),
            system_prompt: Some(
                "You are a conversation summarizer. Produce a brief, factual summary.".into(),
            ),
            messages: summary_messages,
            temperature: Some(0.0),
            max_tokens: Some(1024),
            tools: None,
        };

        let response = self.llm.complete(request).await?;

        let mut result = vec![LlmMessage {
            role: "system".into(),
            content: format!("[Conversation summary]: {}", response.content),
            tool_calls: None,
            tool_call_id: None,
        }];
        result.extend_from_slice(suffix);
        Ok(result)
    }
}

/// A conversation that auto-compacts when approaching the context limit.
pub struct ManagedConversation {
    messages: Vec<LlmMessage>,
    config: ContextConfig,
    counter: Arc<dyn TokenCounter>,
    strategy: Arc<dyn CompactionStrategy>,
}

impl ManagedConversation {
    pub fn new(
        config: ContextConfig,
        counter: Arc<dyn TokenCounter>,
        strategy: Arc<dyn CompactionStrategy>,
    ) -> Self {
        Self {
            messages: Vec::new(),
            config,
            counter,
            strategy,
        }
    }

    pub fn push(&mut self, message: LlmMessage) {
        self.messages.push(message);
    }

    pub fn messages(&self) -> &[LlmMessage] {
        &self.messages
    }

    pub fn estimated_tokens(&self) -> u32 {
        self.counter.count_messages(&self.messages)
    }

    /// Compact if estimated_tokens > config.max_input_tokens * config.compact_threshold.
    pub async fn compact_if_needed(&mut self) -> Result<bool> {
        let threshold =
            (self.config.max_input_tokens as f64 * self.config.compact_threshold) as u32;
        if self.estimated_tokens() <= threshold {
            return Ok(false);
        }
        self.force_compact().await?;
        Ok(true)
    }

    /// Force compaction regardless of token count.
    pub async fn force_compact(&mut self) -> Result<()> {
        let target =
            (self.config.max_input_tokens as f64 * self.config.compact_threshold * 0.7) as u32;
        let compacted = self
            .strategy
            .compact(&self.messages, target, self.counter.as_ref())
            .await?;
        self.messages = compacted;
        Ok(())
    }
}
