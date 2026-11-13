//! [`LlmResponse`]: per-call output from an LLM backend.

use elide_core::backend::Response;
use elide_core::entity::audit::ModelId;

use crate::candidates::Candidates;
use crate::modality::LlmModality;

/// One per-call LLM response from an LLM backend, generic over the
/// modality: the structured candidate batch the backend extracted, and the
/// model that produced it.
///
/// The recognizer localizes each candidate into the source and builds the
/// final entities, each attributed to the model.
#[derive(Debug, Clone, Response)]
pub struct LlmResponse<M: LlmModality> {
    /// The model that produced the candidates.
    #[backend(model)]
    pub model: ModelId,
    /// The structured candidate batch the model produced.
    pub candidates: Candidates<M::Item>,
}

impl<M: LlmModality> LlmResponse<M> {
    /// The `candidates` `model` produced.
    #[must_use]
    pub fn new(model: ModelId, candidates: Candidates<M::Item>) -> Self {
        Self { model, candidates }
    }
}
