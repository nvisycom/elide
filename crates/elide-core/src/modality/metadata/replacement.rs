//! [`MetadataReplacement`]: what a metadata operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;

/// What a metadata operator produces for a field.
///
/// A field is atomic, so the natural treatment is [`Removed`], dropping it
/// entirely. [`Replace`] overwrites the value in place, for a producer that
/// would rather substitute a field (a blanked author, a redacted-marker date)
/// than drop it. Non-exhaustive: a hashing or generalizing treatment can be
/// added without breaking callers.
///
/// [`Removed`]: MetadataReplacement::Removed
/// [`Replace`]: MetadataReplacement::Replace
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum MetadataReplacement {
    /// Drop the field entirely.
    Removed,
    /// Overwrite the field's value with the given text.
    Replace(String),
}

impl ModalityReplacement for MetadataReplacement {}
