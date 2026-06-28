//! [`Annotations<M>`]: the caller's per-modality inclusion and exclusion
//! regions for one analysis.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::Modality;
use crate::recognition::annotation::{Exclusion, Inclusion};

/// Caller-supplied region annotations for a modality `M`, the per-medium
/// companion to the modality-free [`Scope`].
///
/// Regions are `M::Location`-typed (a text byte range, an image bounding
/// box, an audio time span), so unlike the [`Scope`] policy they can't be
/// shared across modalities — they attach to the analyzer of *their*
/// modality. An empty `Annotations` (the default) asserts no regions, the
/// common case.
///
/// [`Scope`]: super::super::Scope
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(bound = "M::Location: schemars::JsonSchema")
)]
pub struct Annotations<M: Modality> {
    /// Caller-supplied candidate regions (each a region the caller believes
    /// may hold an entity, with an optional claimed label, name, and
    /// confidence). Recognizers that adjudicate inclusions (typically
    /// LLM-based) fold these into detection to confirm, relocate, or reject
    /// each one; the rest ignore them.
    pub inclusions: Vec<Inclusion<M>>,
    /// Caller-supplied protected regions. The analyzer drops any entity
    /// whose location overlaps an exclusion, regardless of which recognizer
    /// found it.
    pub exclusions: Vec<Exclusion<M>>,
}

impl<M: Modality> Annotations<M> {
    /// Empty annotations: no regions asserted.
    pub fn new() -> Self {
        Self {
            inclusions: Vec::new(),
            exclusions: Vec::new(),
        }
    }

    /// Add a candidate [`Inclusion`] region.
    #[must_use]
    pub fn with_inclusion(mut self, inclusion: Inclusion<M>) -> Self {
        self.inclusions.push(inclusion);
        self
    }

    /// Replace the inclusion regions with `inclusions`.
    #[must_use]
    pub fn with_inclusions(mut self, inclusions: Vec<Inclusion<M>>) -> Self {
        self.inclusions = inclusions;
        self
    }

    /// Add a protected [`Exclusion`] region.
    #[must_use]
    pub fn with_exclusion(mut self, exclusion: Exclusion<M>) -> Self {
        self.exclusions.push(exclusion);
        self
    }

    /// Replace the exclusion regions with `exclusions`.
    #[must_use]
    pub fn with_exclusions(mut self, exclusions: Vec<Exclusion<M>>) -> Self {
        self.exclusions = exclusions;
        self
    }
}
