//! [`Exclusion<M>`]: a caller-supplied region to leave untouched.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::Modality;

/// Caller-supplied region in which nothing should be flagged.
///
/// An exclusion is a hard "skip here": the analyzer drops any recognized
/// entity whose location overlaps it, regardless of which recognizer
/// found it. Use it to protect a region the caller knows is safe (a
/// public sender address, a fixture block, a header row).
///
/// The opposite direction is an [`Inclusion`], which adds a candidate
/// region; an exclusion only removes. It carries no label, name, or
/// confidence: it asserts the *absence* of anything to redact, not a
/// claim about an entity.
///
/// [`Inclusion`]: super::Inclusion
#[derive(Debug, PartialEq)]
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
pub struct Exclusion<M: Modality> {
    /// Region in modality-native coordinates.
    pub location: M::Location,
}

// Manual `Clone`: `derive` would add a spurious `M: Clone` bound, but `M`
// is a zero-size marker. The location clones via its own bound.
impl<M: Modality> Clone for Exclusion<M> {
    fn clone(&self) -> Self {
        Self {
            location: self.location.clone(),
        }
    }
}

impl<M: Modality> Exclusion<M> {
    /// Exclude the given region.
    #[must_use]
    pub fn new(location: M::Location) -> Self {
        Self { location }
    }
}
