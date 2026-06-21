//! [`LlmResponse`]: per-call output from an [`LlmBackend`].
//!
//! [`LlmBackend`]: super::LlmBackend

use crate::candidates::Candidates;

/// One per-call LLM response from an [`LlmBackend<M>`], generic over the
/// modality.
///
/// Wraps the structured candidate batch the backend extracted. The
/// recognizer localizes each candidate into the source and builds the
/// final entities.
///
/// [`LlmBackend<M>`]: super::LlmBackend
#[derive(Debug, Clone)]
pub struct LlmResponse<M: Candidates> {
    /// The structured candidate batch the model produced.
    pub candidates: M::Batch,
}

impl<M: Candidates> LlmResponse<M> {
    /// Wrap a candidate batch as a response.
    pub fn new(candidates: M::Batch) -> Self {
        Self { candidates }
    }
}

// Hand-written so the bound is `M::Batch: Default` (always true for
// `Candidates`), not the spurious `M: Default` a derive would add.
impl<M: Candidates> Default for LlmResponse<M> {
    fn default() -> Self {
        Self {
            candidates: M::Batch::default(),
        }
    }
}
