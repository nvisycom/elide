//! [`LlmResponse`]: per-call output from an [`LlmBackend`].
//!
//! [`LlmBackend`]: super::LlmBackend

use elide_core::backend::{BackendResponse, Meter, TokenCounts, Units};

use crate::candidates::Candidates;
use crate::modality::LlmModality;

/// One per-call LLM response from an [`LlmBackend<M>`], generic over the
/// modality.
///
/// Wraps the structured candidate batch the backend extracted, plus the tokens
/// the call spent when the provider surfaces them. The recognizer localizes each
/// candidate into the source and builds the final entities; the tokens are read
/// by a `Metered` wrapper, never by the
/// recognizer.
///
/// [`LlmBackend<M>`]: super::LlmBackend
#[derive(Debug, Clone)]
pub struct LlmResponse<M: LlmModality> {
    /// The structured candidate batch the model produced.
    pub candidates: Candidates<M::Item>,
    /// Tokens the call spent, when the backend can surface them from the
    /// provider. [`TokenCounts::default`] (all `None`) when it cannot.
    pub tokens: TokenCounts,
}

impl<M: LlmModality> LlmResponse<M> {
    /// Wrap a candidate batch as a response, with no token counts.
    pub fn new(candidates: Candidates<M::Item>) -> Self {
        Self {
            candidates,
            tokens: TokenCounts::default(),
        }
    }

    /// Attach token counts the backend recovered from the provider.
    #[must_use]
    pub fn with_tokens(mut self, tokens: TokenCounts) -> Self {
        self.tokens = tokens;
        self
    }
}

impl<M: LlmModality> BackendResponse for LlmResponse<M> {
    /// LLMs bill in tokens; an empty count set reports no meter.
    fn units(&self) -> Units {
        if self.tokens.is_empty() {
            Units::none()
        } else {
            Units::from(Meter::Tokens(self.tokens))
        }
    }

    fn output_count(&self) -> Option<u64> {
        Some(self.candidates.entities.len() as u64)
    }
}

// Hand-written so the bound stays `M: LlmModality` (which yields a
// `Default` batch), not the spurious `M: Default` a derive would add.
impl<M: LlmModality> Default for LlmResponse<M> {
    fn default() -> Self {
        Self {
            candidates: Candidates::default(),
            tokens: TokenCounts::default(),
        }
    }
}
