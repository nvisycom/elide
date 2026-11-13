//! [`LlmModality`]: the per-modality binding the LLM path is generic over.
//!
//! Ties a [`Modality`] to its structured candidate item (the `T` rig's
//! `Extractor` produces) and to how that batch becomes entities. The
//! backend names the candidate shape from `M` alone; the recognizer lifts
//! the batch into entities through `M`, so both stay fully generic.

mod image;
mod localize;
mod text;

use elide_core::entity::audit::{AuditEvent, ModelId};
use elide_core::entity::{Entity, EntityCoRef, LabelRef};
use elide_core::modality::Modality;
use elide_core::primitive::Confidence;
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
    /// coordinate space, against the source `data`, each attributed to
    /// `model`.
    fn lift(batch: Candidates<Self::Item>, data: &Self::Data, model: &ModelId)
    -> Vec<Entity<Self>>;
}

const TARGET: &str = "elide_llm::modality";

/// Default confidence assigned to a candidate when the model didn't score
/// it.
const DEFAULT_CONFIDENCE: f64 = 0.5;

/// Builds the entities of one recognizer call: each attributed to the call's
/// model, with the model's coreference handles scoped to the call.
pub(crate) struct Lifter<'a> {
    source: &'static str,
    model: &'a ModelId,
    scope: CorefScope,
}

impl<'a> Lifter<'a> {
    /// A lifter for one call, its audit events naming `source` and `model`.
    pub(crate) fn new(source: &'static str, model: &'a ModelId) -> Self {
        Self {
            source,
            model,
            scope: CorefScope::new(),
        }
    }

    /// The entity for one candidate at `location`, or `None` (with a warning)
    /// when its confidence is NaN. An unscored candidate gets the default
    /// confidence; one out of `0.0..=1.0` is clamped into it.
    pub(crate) fn entity<M: Modality>(
        &self,
        label: String,
        confidence: Option<f64>,
        location: M::Location,
        coreference: Option<&str>,
    ) -> Option<Entity<M>> {
        let raw = confidence.unwrap_or(DEFAULT_CONFIDENCE).clamp(0.0, 1.0);
        let Some(confidence) = Confidence::new(raw as f32) else {
            tracing::warn!(
                target: TARGET,
                label = %label,
                "dropping a candidate whose confidence is not a number",
            );
            return None;
        };
        let event = AuditEvent::model(
            self.source,
            confidence,
            location.clone(),
            self.model.clone(),
        );
        let entity = Entity::new(LabelRef::new(label), location).with_event(event);
        // The model groups mentions of the same real-world entity under a
        // shared handle; scoped to this call, it is the entity's coreference
        // cluster.
        Some(match coreference {
            Some(handle) => entity.with_coref(self.scope.coref(handle)),
            None => entity,
        })
    }
}

/// The namespace of one recognizer call's coreference handles.
///
/// A model names its clusters with handles of its own (`c1`, `person_1`, a
/// name), which repeat across calls. Prefixing each with a fresh id per call
/// keeps the mentions of one call's cluster together while two calls'
/// clusters never share an [`EntityCoRef`], so a surrogate keyed on it never
/// links unrelated entities.
struct CorefScope(Uuid);

impl CorefScope {
    /// A fresh namespace for one call.
    fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// The coreference id of the model's `handle` within this call.
    fn coref(&self, handle: &str) -> EntityCoRef {
        EntityCoRef::new(format!("{}/{handle}", self.0))
    }
}
