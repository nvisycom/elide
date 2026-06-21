//! [`LlmResponse`]: per-call output from an [`LlmBackend`].
//!
//! [`LlmBackend`]: super::LlmBackend

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
}

impl<M: LlmModality> LlmResponse<M> {
    /// Wrap a candidate batch as a response.
    pub fn new(candidates: Candidates<M::Item>) -> Self {
        Self { candidates }
    }
}

// Hand-written so the bound stays `M: LlmModality` (which yields a
// `Default` batch), not the spurious `M: Default` a derive would add.
impl<M: LlmModality> Default for LlmResponse<M> {
    fn default() -> Self {
        Self {
            candidates: Candidates::default(),
        }
    }
}
