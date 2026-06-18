//! [`LlmConfig`]: sampling, retry, context-window, and preamble
//! settings for a [`RigBackend`].
//!
//! [`RigBackend`]: super::RigBackend

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::context::ContextWindow;

/// Sampling, retry, context-window, and preamble settings for a
/// [`RigBackend`].
///
/// [`RigBackend`]: super::RigBackend
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LlmConfig {
    /// Sampling temperature (default: 0.1).
    pub temperature: f64,
    /// Maximum output tokens (default: 4096).
    pub max_tokens: u64,
    /// Maximum retries for transient HTTP errors (default: 3).
    pub max_retries: u32,
    /// Context window for chunking large inputs. When set, prompts
    /// exceeding the input budget are summarised via an extra LLM
    /// call before the real call. See [`compact`].
    ///
    /// [`compact`]: Self::compact
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<ContextWindow>,
    /// Whether to silently compact over-budget prompts. Only fires
    /// when `context_window` is also set. Defaults to `true`.
    #[serde(default = "default_true")]
    pub compact: bool,
    /// System prompt prepended to every request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            temperature: 0.1,
            max_tokens: 4096,
            max_retries: 3,
            context_window: None,
            compact: true,
            preamble: None,
        }
    }
}

fn default_true() -> bool {
    true
}
