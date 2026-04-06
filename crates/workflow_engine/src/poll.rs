// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::llm::{LlmMessage, LlmProvider, LlmRequest};
use crate::workflow::PollPredicate;

/// Result of a poll predicate evaluation: the matched step and its fetched payload.
#[derive(Debug, Clone)]
pub struct PollMatch {
    pub step_name: String,
    pub payload: JsonValue,
}

/// Trait for fetching external resources by type and id.
///
/// Implementations might hit a database, HTTP service, or in-memory cache.
#[async_trait]
pub trait ResourceFetcher: Send + Sync {
    /// Fetch a resource. Returns `Ok(None)` when the resource does not exist yet.
    async fn fetch(&self, resource_type: &str, resource_id: &str) -> Result<Option<JsonValue>>;
}

/// Evaluates a set of PollPredicates against a ResourceFetcher.
///
/// A predicate is considered "matched" when the fetcher returns `Some(payload)`
/// for its `(resource_type, resource_id)` pair.
pub struct PollEvaluator;

impl PollEvaluator {
    pub fn new() -> Self {
        Self
    }

    /// Evaluate all predicates, returning matches for those whose resource is available.
    pub async fn evaluate(
        &self,
        polls: &[PollPredicate],
        fetcher: &dyn ResourceFetcher,
    ) -> Vec<PollMatch> {
        let mut matches = Vec::new();

        for poll in polls {
            debug!(
                "Evaluating poll: step={}, resource_type={}, resource_id={}",
                poll.step_name, poll.resource_type, poll.resource_id
            );

            match fetcher.fetch(&poll.resource_type, &poll.resource_id).await {
                Ok(Some(payload)) => {
                    info!(
                        "Poll matched: step={}, resource_type={}",
                        poll.step_name, poll.resource_type
                    );
                    matches.push(PollMatch {
                        step_name: poll.step_name.clone(),
                        payload,
                    });
                }
                Ok(None) => debug!("Poll not ready: step={}", poll.step_name),
                Err(e) => warn!("Poll fetch error: step={}, error={}", poll.step_name, e),
            }
        }

        matches
    }
}

impl Default for PollEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

/// LLM-based semantic intent router.
///
/// Given a user message and a list of `(step_name, intent_description)` pairs,
/// asks the LLM to classify which intent (if any) the message matches.
///
/// The LLM provider is injected via the [`LlmProvider`] trait, so any backend
/// (OpenAI, Claude, Doubao, Ollama …) can be used.
pub struct IntentRouterV2 {
    llm: Arc<dyn LlmProvider>,
    model: String,
}

impl IntentRouterV2 {
    pub fn new(llm: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        Self {
            llm,
            model: model.into(),
        }
    }

    /// Classify a message against a set of intents.
    ///
    /// Returns the `step_name` of the best-matching intent, or `None` if no
    /// intent matches.
    pub async fn classify(
        &self,
        message: &str,
        intents: &[(String, String)],
    ) -> Result<Option<String>> {
        if intents.is_empty() {
            return Ok(None);
        }

        let intent_list = intents
            .iter()
            .enumerate()
            .map(|(i, (name, desc))| format!("{}. [{}]: {}", i + 1, name, desc))
            .collect::<Vec<_>>()
            .join("\n");

        let system_prompt = "You are an intent classifier. Given a user message and a numbered list of intents, reply with ONLY the step name in square brackets that best matches the message. If no intent matches, reply with NONE.";

        let user_prompt = format!(
            "Intents:\n{}\n\nUser message: \"{}\"\n\nWhich intent matches? Reply with the step name in brackets or NONE.",
            intent_list, message
        );

        debug!(
            "IntentRouterV2: classifying message against {} intents",
            intents.len()
        );

        let response = self
            .llm
            .complete(LlmRequest {
                model: self.model.clone(),
                system_prompt: Some(system_prompt.into()),
                messages: vec![LlmMessage {
                    role: "user".into(),
                    content: user_prompt,
                    tool_calls: None,
                    tool_call_id: None,
                }],
                temperature: Some(0.0),
                max_tokens: Some(64),
                tools: None,
            })
            .await?;

        let raw = response.content.trim();
        info!("IntentRouterV2: LLM response = {:?}", raw);

        for (name, _) in intents {
            if raw.contains(name.as_str()) {
                return Ok(Some(name.clone()));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedFetcher {
        match_type: String,
        payload: JsonValue,
    }

    #[async_trait]
    impl ResourceFetcher for FixedFetcher {
        async fn fetch(&self, resource_type: &str, _: &str) -> Result<Option<JsonValue>> {
            if resource_type == self.match_type {
                Ok(Some(self.payload.clone()))
            } else {
                Ok(None)
            }
        }
    }

    struct EmptyFetcher;
    #[async_trait]
    impl ResourceFetcher for EmptyFetcher {
        async fn fetch(&self, _: &str, _: &str) -> Result<Option<JsonValue>> {
            Ok(None)
        }
    }

    struct ErrorFetcher;
    #[async_trait]
    impl ResourceFetcher for ErrorFetcher {
        async fn fetch(&self, _: &str, _: &str) -> Result<Option<JsonValue>> {
            Err(anyhow::anyhow!("fetch failed"))
        }
    }

    fn make_poll(rt: &str, ri: &str, step: &str) -> PollPredicate {
        PollPredicate {
            resource_type: rt.into(),
            resource_id: ri.into(),
            step_name: step.into(),
            intent_desc: None,
        }
    }

    #[tokio::test]
    async fn evaluate_single_match() {
        let ev = PollEvaluator::new();
        let fetcher = FixedFetcher {
            match_type: "message".into(),
            payload: serde_json::json!({"text": "hello"}),
        };
        let matches = ev
            .evaluate(&[make_poll("message", "c1", "wait_reply")], &fetcher)
            .await;
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].step_name, "wait_reply");
    }

    #[tokio::test]
    async fn evaluate_no_match() {
        let ev = PollEvaluator::new();
        let matches = ev
            .evaluate(&[make_poll("message", "c1", "step")], &EmptyFetcher)
            .await;
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn evaluate_empty_polls() {
        let ev = PollEvaluator::new();
        let matches = ev.evaluate(&[], &EmptyFetcher).await;
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn evaluate_partial_match() {
        let ev = PollEvaluator::new();
        let fetcher = FixedFetcher {
            match_type: "message".into(),
            payload: serde_json::json!({"id": 42}),
        };
        let polls = vec![
            make_poll("message", "c1", "step_a"),
            make_poll("case_state", "c2", "step_b"),
            make_poll("message", "c3", "step_c"),
        ];
        let matches = ev.evaluate(&polls, &fetcher).await;
        assert_eq!(matches.len(), 2);
    }

    #[tokio::test]
    async fn evaluate_error_skips_gracefully() {
        let ev = PollEvaluator::new();
        let matches = ev
            .evaluate(&[make_poll("message", "c1", "step_a")], &ErrorFetcher)
            .await;
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn intent_router_construction() {
        use crate::llm::{LlmResponse, LlmUsage};

        struct EchoLlm;
        #[async_trait]
        impl LlmProvider for EchoLlm {
            async fn complete(&self, req: LlmRequest) -> Result<LlmResponse> {
                Ok(LlmResponse {
                    content: req.messages[0].content.clone(),
                    usage: LlmUsage {
                        prompt_tokens: 1,
                        completion_tokens: 1,
                    },
                    finish_reason: "stop".into(),

                    tool_calls: None,
                })
            }
        }

        let router = IntentRouterV2::new(Arc::new(EchoLlm), "test-model");
        // Just verify construction works — actual LLM calls are tested in integration
        let _ = router;
    }
}
