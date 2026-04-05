use std::sync::Arc;
use workflow_engine::compact::{ManagedConversation, TruncationCompaction};
use workflow_engine::llm::LlmMessage;
use workflow_engine::token::{CharTokenCounter, ContextConfig, TokenCounter};

fn msg(role: &str, content: &str) -> LlmMessage {
    LlmMessage {
        role: role.into(),
        content: content.into(),
        tool_calls: None,
        tool_call_id: None,
    }
}

// ── CharTokenCounter ─────────────────────────────────────────────────────────

#[test]
fn char_counter_counts_text() {
    let counter = CharTokenCounter;
    // "hello" is 5 chars → 5/4 = 1 (integer div, min 1)
    assert_eq!(counter.count_text("hello"), 1);
}

#[test]
fn char_counter_counts_messages() {
    let counter = CharTokenCounter;
    let msgs = vec![
        msg("user", "hello world"),     // 11 chars → 2
        msg("assistant", "hi there!!"), // 10 chars → 2
    ];
    let total = counter.count_messages(&msgs);
    assert_eq!(total, 4); // 2 + 2
}

// ── ContextConfig ────────────────────────────────────────────────────────────

#[test]
fn context_config_construction() {
    let config = ContextConfig {
        max_input_tokens: 100_000,
        compact_threshold: 0.9,
        max_output_tokens: 4_096,
    };
    assert_eq!(config.max_input_tokens, 100_000);
    assert!((config.compact_threshold - 0.9).abs() < f64::EPSILON);
    assert_eq!(config.max_output_tokens, 4_096);
}

#[test]
fn context_config_default() {
    let config = ContextConfig::default();
    assert_eq!(config.max_input_tokens, 128_000);
    assert!((config.compact_threshold - 0.85).abs() < f64::EPSILON);
    assert_eq!(config.max_output_tokens, 8_192);
}

// ── ManagedConversation ──────────────────────────────────────────────────────

#[tokio::test]
async fn managed_conversation_push_and_get() {
    let config = ContextConfig {
        max_input_tokens: 100_000,
        compact_threshold: 0.85,
        max_output_tokens: 8_192,
    };
    let counter: Arc<dyn TokenCounter> = Arc::new(CharTokenCounter);
    let strategy = Arc::new(TruncationCompaction { preserve_recent: 2 });

    let mut conv = ManagedConversation::new(config, counter, strategy);
    conv.push(msg("user", "hello"));
    conv.push(msg("assistant", "hi there"));

    assert_eq!(conv.messages().len(), 2);
    assert_eq!(conv.messages()[0].role, "user");
    assert_eq!(conv.messages()[1].content, "hi there");
}

#[tokio::test]
async fn managed_conversation_compact_if_needed_no_op() {
    let config = ContextConfig {
        max_input_tokens: 100_000,
        compact_threshold: 0.85,
        max_output_tokens: 8_192,
    };
    let counter: Arc<dyn TokenCounter> = Arc::new(CharTokenCounter);
    let strategy = Arc::new(TruncationCompaction { preserve_recent: 2 });

    let mut conv = ManagedConversation::new(config, counter, strategy);
    conv.push(msg("user", "short"));

    let compacted = conv.compact_if_needed().await.unwrap();
    assert!(!compacted); // under threshold, no compaction needed
    assert_eq!(conv.messages().len(), 1);
}

#[tokio::test]
async fn managed_conversation_compact_if_needed_triggers() {
    // Very small max_input_tokens so a few messages exceed the threshold
    let config = ContextConfig {
        max_input_tokens: 10,   // 10 tokens max
        compact_threshold: 0.5, // trigger at 5 tokens
        max_output_tokens: 100,
    };
    let counter: Arc<dyn TokenCounter> = Arc::new(CharTokenCounter);
    let strategy = Arc::new(TruncationCompaction { preserve_recent: 1 });

    let mut conv = ManagedConversation::new(config, counter, strategy);
    // Each message of 40 chars → 10 tokens each. Total > threshold of 5
    conv.push(msg("user", "a]".repeat(20).as_str()));
    conv.push(msg("assistant", "b]".repeat(20).as_str()));
    conv.push(msg("user", "c]".repeat(20).as_str()));

    let compacted = conv.compact_if_needed().await.unwrap();
    assert!(compacted);
    // After compaction, only preserve_recent (1) messages should remain
    assert!(conv.messages().len() <= 1);
}

// ── TruncationCompaction ─────────────────────────────────────────────────────

#[tokio::test]
async fn truncation_compaction_preserves_recent() {
    use workflow_engine::compact::CompactionStrategy;

    let strategy = TruncationCompaction { preserve_recent: 2 };
    let counter = CharTokenCounter;

    let messages = vec![
        msg("user", "first message that is old"),
        msg("assistant", "second old message"),
        msg("user", "recent one"),
        msg("assistant", "latest"),
    ];

    // Target 5 tokens — very small. Should keep last 2 regardless.
    let result = strategy.compact(&messages, 5, &counter).await.unwrap();
    assert!(result.len() >= 2);
    assert_eq!(result.last().unwrap().content, "latest");
}

#[tokio::test]
async fn truncation_compaction_removes_oldest() {
    use workflow_engine::compact::CompactionStrategy;

    let strategy = TruncationCompaction { preserve_recent: 1 };
    let counter = CharTokenCounter;

    let messages = vec![
        msg("user", "old message one"),
        msg("assistant", "old message two"),
        msg("user", "old message three"),
        msg("assistant", "keep this one"),
    ];

    // Target very small so oldest messages get dropped
    let result = strategy.compact(&messages, 5, &counter).await.unwrap();
    assert_eq!(result.last().unwrap().content, "keep this one");
    assert!(result.len() < messages.len());
}
