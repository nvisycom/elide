//! [`EntityCoRef`]: the identifier coreferent entities share.

use std::fmt;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Coreference identifier shared by entities that denote the same
/// real-world thing.
///
/// Assigned by a recognizer that performs coreference resolution: an
/// NER model or LLM that recognises "Alice", "she", and "Ms. Smith" as
/// the same person within one detection call. Two entities carrying the
/// same `EntityCoRef` are coreferent: they refer to the same underlying
/// entity even though each is a distinct mention with its own span.
///
/// The identifier is opaque and only meaningful *within* a single
/// detection call: it is the recognizer's local handle for a cluster of
/// mentions, not a global key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct EntityCoRef(#[cfg_attr(feature = "schema", schemars(with = "String"))] HipStr<'static>);

impl EntityCoRef {
    /// Coreference identifier from a recognizer-assigned handle.
    pub fn new(id: impl Into<HipStr<'static>>) -> Self {
        Self(id.into())
    }

    /// Identifier as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for EntityCoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}
