//! [`LlmModality`]: the per-modality binding the LLM path is generic over.
//!
//! Ties a [`Modality`] to its structured candidate item (the `T` rig's
//! `Extractor` produces) and to how that batch becomes entities. The
//! backend names the candidate shape from `M` alone; the recognizer lifts
//! the batch into entities through `M`, so both stay fully generic.

mod image;
mod localize;
mod text;

use elide_core::entity::{Entity, EntityCoRef};
use elide_core::modality::Modality;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::candidates::Candidates;

/// Per-modality binding the LLM recognizer is generic over.
///
/// Ties a modality to the candidate item the model produces and to how a
/// candidate batch lifts into entities.
pub trait LlmModality: Modality + Sized {
    /// The per-candidate item the model fills in (e.g. a text candidate or
    /// an image candidate). [`Candidates<Self::Item>`] is the structured
    /// shape rig's `Extractor` is constrained to.
    type Item: JsonSchema + for<'a> Deserialize<'a> + Serialize + Send + Sync + 'static;

    /// Lift an extracted candidate batch into entities, in this modality's
    /// coordinate space, against the source `data`.
    fn lift(batch: Candidates<Self::Item>, data: &Self::Data) -> Vec<Entity<Self>>;
}

/// Default confidence assigned to a candidate when the model didn't score
/// it.
pub(crate) const DEFAULT_CONFIDENCE: f64 = 0.5;

/// The namespace of one recognizer call's coreference handles.
///
/// A model names its clusters with handles of its own (`c1`, `person_1`, a
/// name), which repeat across calls. Prefixing each with a fresh id per call
/// keeps the mentions of one call's cluster together while two calls'
/// clusters never share an [`EntityCoRef`], so a surrogate keyed on it never
/// links unrelated entities.
pub(crate) struct CorefScope(Uuid);

impl CorefScope {
    /// A fresh namespace for one call.
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// The coreference id of the model's `handle` within this call.
    pub(crate) fn coref(&self, handle: &str) -> EntityCoRef {
        EntityCoRef::new(format!("{}/{handle}", self.0))
    }
}
