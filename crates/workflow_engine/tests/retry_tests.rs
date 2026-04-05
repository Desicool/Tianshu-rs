use anyhow::anyhow;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use workflow_engine::llm::{LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmUsage};
use workflow_engine::retry::{with_retry, ErrorClass, RetryPolicy};
use workflow_engine::ResilientLlmProvider;

// ── Retry core tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn retry_succeeds_on_first_attempt() {
    let policy = RetryPolicy::default_transient();
    let result = with_retry(&policy, |_ctx| async { Ok::<_, anyhow::Error>(42) }).await;
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn retry_succeeds_after_transient_failure() {
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = call_count.clone();

    let policy = RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
        backoff_factor: 2.0,
        classify: Arc::new(|_| ErrorClass::Transient),
    };

    let result = with_retry(&policy, |_ctx| {
        let cc = cc.clone();
        async move {
            let n = cc.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err(anyhow!("transient failure"))
            } else {
                Ok(99)
            }
        }
    })
    .await;

    assert_eq!(result.unwrap(), 99);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retry_fatal_error_not_retried() {
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = call_count.clone();

    let policy = RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
        backoff_factor: 2.0,
        classify: Arc::new(|_| ErrorClass::Fatal),
    };

    let result = with_retry(&policy, |_ctx| {
        let cc = cc.clone();
        async move {
            cc.fetch_add(1, Ordering::SeqCst);
            Err::<i32, _>(anyhow!("fatal"))
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_max_attempts_exceeded() {
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = call_count.clone();

    let policy = RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
        backoff_factor: 2.0,
        classify: Arc::new(|_| ErrorClass::Transient),
    };

    let result = with_retry(&policy, |_ctx| {
        let cc = cc.clone();
        async move {
            cc.fetch_add(1, Ordering::SeqCst);
            Err::<i32, _>(anyhow!("keep failing"))
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn retry_backoff_increases() {
    let attempts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let att = attempts.clone();

    let policy = RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_secs(10),
        backoff_factor: 2.0,
        classify: Arc::new(|_| ErrorClass::Transient),
    };

    let _ = with_retry(&policy, |ctx| {
        let att = att.clone();
        async move {
            att.lock().unwrap().push(ctx.attempt);
            Err::<i32, _>(anyhow!("fail"))
        }
    })
    .await;

    let recorded = attempts.lock().unwrap().clone();
    assert_eq!(recorded, vec![0, 1, 2, 3]);
}

// ── ResilientLlmProvider tests ───────────────────────────────────────────────

fn make_request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        system_prompt: None,
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "hello".into(),
            tool_calls: None,
            tool_call_id: None,
        }],
        temperature: None,
        max_tokens: Some(100),
        tools: None,
    }
}

fn ok_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: content.into(),
        usage: LlmUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
        },
        finish_reason: "stop".into(),
        tool_calls: None,
    }
}

struct FixedProvider {
    response: LlmResponse,
}

#[async_trait]
impl LlmProvider for FixedProvider {
    async fn complete(&self, _request: LlmRequest) -> anyhow::Result<LlmResponse> {
        Ok(self.response.clone())
    }
}

struct FailProvider {
    message: String,
}

#[async_trait]
impl LlmProvider for FailProvider {
    async fn complete(&self, _request: LlmRequest) -> anyhow::Result<LlmResponse> {
        Err(anyhow!("{}", self.message))
    }
}

struct CountingFailThenOkProvider {
    call_count: AtomicU32,
    fail_times: u32,
    fail_message: String,
    ok_response: LlmResponse,
}

#[async_trait]
impl LlmProvider for CountingFailThenOkProvider {
    async fn complete(&self, _request: LlmRequest) -> anyhow::Result<LlmResponse> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_times {
            Err(anyhow!("{}", self.fail_message))
        } else {
            Ok(self.ok_response.clone())
        }
    }
}

#[tokio::test]
async fn resilient_provider_falls_back_on_overloaded() {
    let primary = Arc::new(FailProvider {
        message: "overloaded".into(),
    });
    let fallback = Arc::new(FixedProvider {
        response: ok_response("fallback response"),
    });

    let policy = RetryPolicy {
        max_attempts: 2,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
        backoff_factor: 1.0,
        classify: Arc::new(|e| {
            if e.to_string().contains("overloaded") {
                ErrorClass::ProviderOverloaded
            } else {
                ErrorClass::Fatal
            }
        }),
    };

    let resilient = ResilientLlmProvider::new(primary, policy).with_fallback(fallback);

    let resp = resilient.complete(make_request()).await.unwrap();
    assert_eq!(resp.content, "fallback response");
}

#[tokio::test]
async fn resilient_provider_escalates_max_tokens() {
    // Provider that fails with "max_output_tokens" on first call, succeeds on second
    let provider = Arc::new(CountingFailThenOkProvider {
        call_count: AtomicU32::new(0),
        fail_times: 1,
        fail_message: "max_output_tokens".into(),
        ok_response: ok_response("completed"),
    });

    let policy = RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(10),
        backoff_factor: 1.0,
        classify: Arc::new(|e| {
            if e.to_string().contains("max_output_tokens") {
                ErrorClass::MaxOutputTokens
            } else {
                ErrorClass::Fatal
            }
        }),
    };

    let resilient = ResilientLlmProvider::new(provider, policy);

    let resp = resilient.complete(make_request()).await.unwrap();
    assert_eq!(resp.content, "completed");
}
