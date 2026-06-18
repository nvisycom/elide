//! Token budget estimation and prompt compaction trigger.
//!
//! [`ContextWindow`] provides a script-aware heuristic (~4 chars/token
//! for Latin, ~1 token per CJK/emoji character) to decide whether
//! text fits within a model's input budget. When it doesn't, the
//! [`RigBackend`] sends an extra LLM call to summarise the prompt
//! before the real call.
//!
//! [`RigBackend`]: super::RigBackend

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Token budget manager for a single model context window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextWindow {
    /// Maximum tokens the model supports.
    max_tokens: usize,
    /// Tokens reserved for the output/completion.
    reserve_output: usize,
}

impl ContextWindow {
    /// Build a context window from the model's maximum token budget
    /// and the share reserved for the completion.
    pub fn new(max_tokens: usize, reserve_output: usize) -> Self {
        Self {
            max_tokens,
            reserve_output,
        }
    }

    /// Rough token count.
    ///
    /// Uses character count (not byte length) so that CJK, Arabic,
    /// emoji, and other multi-byte scripts are estimated more
    /// accurately. The heuristic assumes ~4 characters per token for
    /// Latin text; CJK and other ideographic characters are counted
    /// as ~1 token each.
    pub fn estimate_tokens(text: &str) -> usize {
        let mut latin_chars = 0usize;
        let mut wide_chars = 0usize;
        for ch in text.chars() {
            if ch.is_ascii() || ch.len_utf8() <= 2 {
                latin_chars += 1;
            } else {
                wide_chars += 1;
            }
        }
        latin_chars.div_ceil(4) + wide_chars
    }

    /// Input token budget (`max_tokens − reserve_output`).
    pub(crate) fn input_budget(&self) -> usize {
        self.max_tokens.saturating_sub(self.reserve_output)
    }

    /// Whether `text` fits within the input budget.
    pub fn fits(&self, text: &str) -> bool {
        Self::estimate_tokens(text) <= self.input_budget()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_within_budget() {
        let cw = ContextWindow::new(100, 20);
        assert!(cw.fits(&"a".repeat(300)));
        assert!(!cw.fits(&"a".repeat(400)));
    }
}
