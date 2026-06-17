//! Audit records: the per-entity [`Provenance`] ledger and the
//! run-level [`Manifest`].
//!
//! [`Provenance`]: crate::provenance::Provenance

mod manifest;

pub use self::manifest::Manifest;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::Modality;
use crate::recognition::{Detection, Merge};

/// The detection audit trail of an [`Entity`].
///
/// This is the model's answer to "full provenance" — the spine that
/// Presidio lacks. Where Presidio keeps a shallow, optional, per-stage
/// explanation that is stripped by default, a `Provenance` is always
/// present and records how the entity came to be:
///
/// 1. **Detections** — every layer that independently found this
///    entity, each with its own location, score, and reasoning. More
///    than one entry means several recognizers (e.g. a pattern *and* a
///    model) agreed, and were merged.
/// 2. **Merge** — if the entity was born from combining detections,
///    the record of *how* (which strategy, what resulting score).
///
/// Provenance covers **detection only** — it freezes once the entity is
/// assembled. Redaction is a separate concern: an operator produces a
/// [`Replacement`](crate::modality::Modality::Replacement), and the
/// audit of *what was hidden, how* is assembled one layer up (by
/// `veil-toolkit` / the orchestrating engine), not stored on the entity.
///
/// Generic over the modality `M` because the retained detections
/// carry their own (per-layer) locations.
///
/// [`Entity`]: crate::entity::Entity
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
pub struct Provenance<M: Modality> {
    /// Every detection layer that found this entity.
    pub detections: Vec<Detection<M>>,
    /// How the detections were combined, if more than one contributed.
    pub merge: Option<Merge>,
}

impl<M: Modality> Provenance<M> {
    /// Provenance for an entity that came straight from a single
    /// detection, with no merge.
    pub fn single(detection: Detection<M>) -> Self {
        Self {
            detections: vec![detection],
            merge: None,
        }
    }

    /// Provenance for an entity born from fusing several detections.
    ///
    /// Retains every contributing detection and records the [`Merge`]
    /// event. Written by the fusion step in `veil-toolkit`.
    pub fn merged(detections: Vec<Detection<M>>, merge: Merge) -> Self {
        Self {
            detections,
            merge: Some(merge),
        }
    }
}
