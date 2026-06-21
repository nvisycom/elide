//! [`LlmConfig`]: sampling, retry, and preamble settings for a
//! [`RigBackend`].
//!
//! [`RigBackend`]: super::RigBackend

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Sampling, retry, and preamble settings for a [`RigBackend`].
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
            preamble: None,
        }
    }
}
