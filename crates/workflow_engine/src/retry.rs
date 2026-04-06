// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Classification of errors for retry decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Network/service blip — retry with backoff.
    Transient,
    /// LLM context window exceeded — needs compaction before retry.
    PromptTooLong,
    /// LLM hit max output tokens — escalate max_tokens.
    MaxOutputTokens,
    /// Provider busy/rate-limited — try fallback provider.
    ProviderOverloaded,
    /// Non-recoverable — stop immediately.
    Fatal,
}

/// Context passed to each retry attempt.
pub struct RetryContext {
    /// 0-based attempt number.
    pub attempt: u32,
    /// Error class from the previous attempt, if any.
    pub last_error_class: Option<ErrorClass>,
}

/// Configuration for retry behavior.
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub backoff_factor: f64,
    pub classify: Arc<dyn Fn(&anyhow::Error) -> ErrorClass + Send + Sync>,
}

impl RetryPolicy {
    /// Default policy: 3 attempts, 100ms→2s backoff, all errors treated as Transient.
    pub fn default_transient() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
            backoff_factor: 2.0,
            classify: Arc::new(|_| ErrorClass::Transient),
        }
    }
}

/// Execute `f` with retries according to `policy`.
///
/// The closure receives a `RetryContext` with the current attempt number and
/// the error class from the previous attempt (if any). Fatal errors and
/// exceeding `max_attempts` cause immediate return of the last error.
pub async fn with_retry<T, F, Fut>(policy: &RetryPolicy, f: F) -> Result<T>
where
    F: Fn(RetryContext) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0u32;
    let mut last_class = None;

    loop {
        let ctx = RetryContext {
            attempt,
            last_error_class: last_class,
        };
        match f(ctx).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let class = (policy.classify)(&e);
                if class == ErrorClass::Fatal || attempt + 1 >= policy.max_attempts {
                    return Err(e);
                }
                let delay_ms = (policy.base_delay.as_millis() as f64
                    * policy.backoff_factor.powi(attempt as i32))
                    as u64;
                let delay =
                    Duration::from_millis(delay_ms.min(policy.max_delay.as_millis() as u64));
                sleep(delay).await;
                last_class = Some(class);
                attempt += 1;
            }
        }
    }
}

impl std::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorClass::Transient => write!(f, "Transient"),
            ErrorClass::PromptTooLong => write!(f, "PromptTooLong"),
            ErrorClass::MaxOutputTokens => write!(f, "MaxOutputTokens"),
            ErrorClass::ProviderOverloaded => write!(f, "ProviderOverloaded"),
            ErrorClass::Fatal => write!(f, "Fatal"),
        }
    }
}
