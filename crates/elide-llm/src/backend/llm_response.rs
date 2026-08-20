//! [`LlmResponse`]: per-call output from an [`LlmBackend`].
//!
//! [`LlmBackend`]: super::LlmBackend

#[cfg(feature = "usage")]
use elide_core::recognition::TokenCounts;

use crate::candidates::Candidates;
use crate::modality::LlmModality;

/// One per-call LLM response from an [`LlmBackend<M>`], generic over the
/// modality.
///
/// Wraps the structured candidate batch the backend extracted. The
/// recognizer localizes each candidate into the source and builds the
/// final entities.
///
/// [`LlmBackend<M>`]: super::LlmBackend
#[derive(Debug, Clone)]
pub struct LlmResponse<M: LlmModality> {
    /// The structured candidate batch the model produced.
    pub candidates: Candidates<M::Item>,
    /// Tokens the call spent, when the backend can surface them from the
    /// provider. [`TokenCounts::default`] (all `None`) when it cannot — some
    /// providers omit usage, and the rig extractor path may not expose it.
    #[cfg(feature = "usage")]
    pub tokens: TokenCounts,
}

impl<M: LlmModality> LlmResponse<M> {
    /// Wrap a candidate batch as a response, with no token counts.
    pub fn new(candidates: Candidates<M::Item>) -> Self {
        Self {
            candidates,
            #[cfg(feature = "usage")]
            tokens: TokenCounts::default(),
        }
    }

    /// Attach token counts the backend recovered from the provider.
    #[cfg(feature = "usage")]
    #[must_use]
    pub fn with_tokens(mut self, tokens: TokenCounts) -> Self {
        self.tokens = tokens;
        self
    }
}

// Hand-written so the bound stays `M: LlmModality` (which yields a
// `Default` batch), not the spurious `M: Default` a derive would add.
impl<M: LlmModality> Default for LlmResponse<M> {
    fn default() -> Self {
        Self {
            candidates: Candidates::default(),
            #[cfg(feature = "usage")]
            tokens: TokenCounts::default(),
        }
    }
}
