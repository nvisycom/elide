//! [`Inclusion<M>`]: a caller-supplied region that may hold an entity.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::LabelRef;
use crate::modality::Modality;
use crate::primitive::Confidence;

/// Caller-supplied region the caller believes may hold an entity.
///
/// An inclusion marks where to look, and optionally claims an entity
/// label, a display name, and a confidence for the claim. Recognizers
/// that support adjudication (typically LLM-based ones) fold inclusions
/// into their detection pass so the model can confirm, relocate, or
/// implicitly reject each one alongside open-ended discovery. Recognizers
/// that don't (pattern, dictionary, generic NER backends) ignore them.
///
/// The opposite direction is an [`Exclusion`], which removes any entity
/// found in a region; an inclusion only adds a candidate.
///
/// `Inclusion<M>` mirrors [`Entity<M>`] structurally: both carry a
/// modality-native location. The difference is direction. An entity is a
/// recognizer's *output*, with a confidence and a provenance trail; an
/// inclusion is a recognizer's *input*.
///
/// [`Exclusion`]: super::Exclusion
/// [`Entity<M>`]: crate::entity::Entity
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(bound = "M::Location: schemars::JsonSchema"))]
pub struct Inclusion<M: Modality> {
    /// Region in modality-native coordinates.
    pub location: M::Location,
    /// Caller-claimed label. When set, recognizers that confirm the
    /// inclusion stamp this on the emitted entity's label.
    pub label: Option<LabelRef>,
    /// Caller-supplied display name. Recognizers that confirm or relocate
    /// this inclusion forward the name into the emitted entity's
    /// provenance.
    pub name: Option<String>,
    /// Caller's confidence in the claim, forwarded to detectors that honor
    /// it (e.g. an LLM prompt) and recorded on a confirmed entity.
    pub confidence: Option<Confidence>,
}

// Manual `Clone`: `derive` would add a spurious `M: Clone` bound, but `M`
// is a zero-size marker. The fields clone via the location's own bound.
impl<M: Modality> Clone for Inclusion<M> {
    fn clone(&self) -> Self {
        Self {
            location: self.location.clone(),
            label: self.label.clone(),
            name: self.name.clone(),
            confidence: self.confidence,
        }
    }
}

impl<M: Modality> Inclusion<M> {
    /// Inclusion with only the location set; label, name, and confidence
    /// default to `None`.
    #[must_use]
    pub fn new(location: M::Location) -> Self {
        Self {
            location,
            label: None,
            name: None,
            confidence: None,
        }
    }

    /// Attach a caller-claimed label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<LabelRef>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Attach a caller-supplied display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Attach the caller's confidence in the claim.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }
}
