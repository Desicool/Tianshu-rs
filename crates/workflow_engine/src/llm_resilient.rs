// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::llm::{LlmProvider, LlmRequest, LlmResponse};
use crate::retry::{with_retry, ErrorClass, RetryContext, RetryPolicy};

type PromptTooLongHandler = Arc<dyn Fn(&mut LlmRequest) + Send + Sync>;

/// An `LlmProvider` that adds retry logic, fallback providers, and
/// automatic max_tokens escalation.
pub struct ResilientLlmProvider {
    primary: Arc<dyn LlmProvider>,
    fallbacks: Vec<Arc<dyn LlmProvider>>,
    retry_policy: RetryPolicy,
    /// Called when prompt-too-long error occurs, before retry.
    on_prompt_too_long: Option<PromptTooLongHandler>,
}

impl ResilientLlmProvider {
    pub fn new(primary: Arc<dyn LlmProvider>, retry_policy: RetryPolicy) -> Self {
        Self {
            primary,
            fallbacks: Vec::new(),
            retry_policy,
            on_prompt_too_long: None,
        }
    }

    pub fn with_fallback(mut self, fallback: Arc<dyn LlmProvider>) -> Self {
        self.fallbacks.push(fallback);
        self
    }

    pub fn with_prompt_too_long_handler(
        mut self,
        handler: impl Fn(&mut LlmRequest) + Send + Sync + 'static,
    ) -> Self {
        self.on_prompt_too_long = Some(Arc::new(handler));
        self
    }
}

/// Maximum ceiling for max_tokens escalation.
const MAX_TOKENS_CEILING: u32 = 128_000;

#[async_trait]
impl LlmProvider for ResilientLlmProvider {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let mut current_request = request;

        let result = with_retry(&self.retry_policy, |ctx: RetryContext| {
            let req = current_request.clone();
            let primary = self.primary.clone();
            let fallbacks = self.fallbacks.clone();
            async move {
                // On ProviderOverloaded from a previous attempt, try fallbacks
                if ctx.last_error_class == Some(ErrorClass::ProviderOverloaded) {
                    let fallback_idx = (ctx.attempt as usize).saturating_sub(1);
                    if fallback_idx < fallbacks.len() {
                        return fallbacks[fallback_idx].complete(req).await;
                    }
                }
                primary.complete(req).await
            }
        })
        .await;

        // If the retry loop exhausted with ProviderOverloaded and we have
        // untried fallbacks, give them a shot outside the loop.
        // The retry loop already tried fallbacks[0..max_attempts-2] (one per attempt
        // starting at attempt 1, via fallback_idx = attempt - 1). Skip those and
        // only try fallbacks that were not yet attempted inside the loop.
        if let Err(ref e) = result {
            let class = (self.retry_policy.classify)(e);
            if class == ErrorClass::ProviderOverloaded {
                let tried_in_loop = (self.retry_policy.max_attempts as usize).saturating_sub(1);
                for fb in self.fallbacks.iter().skip(tried_in_loop) {
                    if let Ok(resp) = fb.complete(current_request.clone()).await {
                        return Ok(resp);
                    }
                }
            }
            if class == ErrorClass::MaxOutputTokens {
                // Escalate max_tokens and retry once more with primary
                let current_max = current_request.max_tokens.unwrap_or(4_096);
                let new_max = (current_max * 2).min(MAX_TOKENS_CEILING);
                current_request.max_tokens = Some(new_max);
                return self.primary.complete(current_request).await;
            }
            if class == ErrorClass::PromptTooLong {
                if let Some(ref handler) = self.on_prompt_too_long {
                    handler(&mut current_request);
                    return self.primary.complete(current_request).await;
                }
            }
        }

        result
    }
}