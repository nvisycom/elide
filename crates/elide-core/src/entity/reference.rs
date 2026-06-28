//! Lightweight references that point at an entity.

use std::fmt;

use hipstr::HipStr;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lightweight, stable reference to an [`Entity`] by its identity.
///
/// Carries only the entity's [`id`], so other records can point at an
/// entity without duplicating it. Mirrors [`LabelRef`]: the full entity
/// is resolved elsewhere, the reference is cheap to copy and store.
///
/// [`Entity`]: crate::entity::Entity
/// [`id`]: crate::entity::Entity::id
/// [`LabelRef`]: crate::entity::LabelRef
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct EntityRef(Uuid);

impl EntityRef {
    /// Reference an entity by its id.
    pub const fn new(id: Uuid) -> Self {
        Self(id)
    }

    /// Referenced entity's id.
    pub const fn id(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for EntityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

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
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
