use crate::llm::LlmMessage;

/// Trait for estimating token counts.
pub trait TokenCounter: Send + Sync {
    fn count_messages(&self, messages: &[LlmMessage]) -> u32;
    fn count_text(&self, text: &str) -> u32;
}

/// Simple chars/4 heuristic token counter.
pub struct CharTokenCounter;

impl TokenCounter for CharTokenCounter {
    fn count_text(&self, text: &str) -> u32 {
        (text.len() as u32).saturating_div(4).max(1)
    }

    fn count_messages(&self, messages: &[LlmMessage]) -> u32 {
        messages.iter().map(|m| self.count_text(&m.content)).sum()
    }
}

/// Configuration for context window management.
#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub max_input_tokens: u32,
    /// Fraction of max_input_tokens at which auto-compaction triggers.
    pub compact_threshold: f64,
    pub max_output_tokens: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_input_tokens: 128_000,
            compact_threshold: 0.85,
            max_output_tokens: 8_192,
        }
    }
}
