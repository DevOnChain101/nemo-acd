use nemo_providers::{Block, Content, Message};
use serde_json::Value;

/// Pluggable token estimator. The default is the Node approximation (chars / 4). `Send + Sync`
/// so compaction futures that hold `&dyn TokenEstimator` can cross `.await` on any runtime.
pub trait TokenEstimator: Send + Sync {
    fn estimate(&self, text: &str) -> u64;
}

#[derive(Debug, Clone, Copy)]
pub struct CharsPerToken(pub u64);

impl Default for CharsPerToken {
    fn default() -> Self {
        Self(4)
    }
}

impl TokenEstimator for CharsPerToken {
    fn estimate(&self, text: &str) -> u64 {
        (text.chars().count() as u64).div_ceil(self.0.max(1))
    }
}

/// Flat cost of an image block (base64 payloads must not be counted as text).
pub const IMAGE_TOKENS: u64 = 1500;

pub fn block_tokens(est: &dyn TokenEstimator, b: &Block) -> u64 {
    match b {
        Block::Text { text } => est.estimate(text),
        Block::Image { .. } => IMAGE_TOKENS,
        Block::ToolUse { name, input, .. } => {
            est.estimate(name) + input.as_ref().map_or(0, |v| est.estimate(&v.to_string()))
        }
        Block::ToolResult { content, .. } => match content {
            None => 0,
            Some(Value::String(s)) => est.estimate(s),
            Some(v) => est.estimate(&v.to_string()),
        },
        Block::Thinking { thinking, .. } => est.estimate(thinking),
        Block::RedactedThinking { data } => est.estimate(data),
    }
}

pub fn message_tokens(est: &dyn TokenEstimator, m: &Message) -> u64 {
    match &m.content {
        Content::Text(t) => est.estimate(t),
        Content::Blocks(bs) => bs.iter().map(|b| block_tokens(est, b)).sum(),
    }
}

pub fn messages_tokens(est: &dyn TokenEstimator, ms: &[Message]) -> u64 {
    ms.iter().map(|m| message_tokens(est, m)).sum()
}
