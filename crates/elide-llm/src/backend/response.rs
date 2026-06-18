//! [`LlmResponse`]: per-call output from an [`LlmBackend`].
//!
//! [`LlmBackend`]: super::LlmBackend

/// One per-call LLM response from a [`LlmBackend`].
///
/// Wraps the model's reply text verbatim. The recognizer
/// deserialises it (and applies whatever markdown-fence /
/// sentinel-text forgiveness its [`Prompt<M>`] needs) on the way
/// out.
///
/// [`LlmBackend`]: super::LlmBackend
/// [`Prompt<M>`]: crate::Prompt
#[derive(Debug, Clone, Default)]
pub struct LlmResponse {
    /// Model's reply text, verbatim.
    pub text: String,
}

impl LlmResponse {
    /// Construct a response from raw text.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}
