//! [`LlmModality`]: the per-modality binding the LLM path is generic over.
//!
//! Ties a [`Modality`] to its structured candidate item (the `T` rig's
//! `Extractor` produces) and to how that batch becomes entities. The
//! backend names the candidate shape from `M` alone; the recognizer lifts
//! the batch into entities through `M`, so both stay fully generic.

mod image;
mod localize;
mod text;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::candidates::Candidates;

/// Per-modality binding for the LLM recognizer: the candidate item the
/// model produces, and how a candidate batch lifts into entities.
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
