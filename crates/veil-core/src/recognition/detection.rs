//! The [`Detection`] — one layer's finding, with its own audit detail.

use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::LabelRef;
use crate::modality::Modality;
use crate::primitive::Confidence;
use crate::recognition::{Explanation, RecognizerId};

/// The record of one recognizer's contribution to an [`Entity`].
///
/// A recognizer emits an entity carrying one `Detection` — its own
/// finding (recognizer id, label, location, confidence, reasoning). The
/// *same* underlying information may be found by several recognizers — a
/// regex pattern *and* an NER model, say — and when the fusion step (in
/// `veil-toolkit`) combines their entities, every contributing
/// `Detection` is unioned into the survivor's provenance. Nothing about
/// any layer's finding is lost.
///
/// Generic over the [`Modality`] `M` so each detection records *exactly*
/// what its layer matched (at `M::Location`) — full audit fidelity.
///
/// [`Entity`]: crate::entity::Entity
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
pub struct Detection<M: Modality> {
    /// Which recognizer (name + version) produced this detection.
    pub recognizer: RecognizerId,
    /// What kind of sensitive information was detected.
    pub label: LabelRef,
    /// Where this layer matched, in its own coordinates.
    pub location: M::Location,
    /// This layer's confidence, before merging.
    pub confidence: Confidence,
    /// Why this layer believed it saw an entity.
    pub explanation: Explanation,
    /// When the detection was made (UTC).
    pub at: Timestamp,
}

impl<M: Modality> Detection<M> {
    /// Record a detection, stamping it with the current time.
    pub fn new(
        recognizer: RecognizerId,
        label: LabelRef,
        location: M::Location,
        confidence: Confidence,
        explanation: Explanation,
    ) -> Self {
        Self {
            recognizer,
            label,
            location,
            confidence,
            explanation,
            at: Timestamp::now(),
        }
    }
}
