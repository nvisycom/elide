//! [`Category`]: a coarse grouping a [`Label`] belongs to.
//!
//! [`Label`]: super::Label

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The coarse group a [`Label`] belongs to, for organizing detected entities
/// by kind (`financial`, `health`, `identity`, …).
///
/// A category answers "what *sort* of information is this" at a display level:
/// a consumer groups a redaction audit into sections by category. It is
/// distinct from a label's [`tags`], which are cross-cutting sensitivity
/// markers (`pii`, `phi`, `pci`) a label may carry several of; a label has at
/// most **one** category. Built-in labels ship with a category; a custom
/// [`Label`] has none unless one is set.
///
/// The value is an open, lowercase `snake_case` identifier, so a custom label
/// can define its own category rather than being confined to the shipped set.
///
/// [`Label`]: super::Label
/// [`tags`]: super::Label::tags
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct Category(#[cfg_attr(feature = "schema", schemars(with = "String"))] HipStr<'static>);

impl Category {
    /// A category from an id (e.g. `"financial"`).
    pub fn new(id: impl Into<HipStr<'static>>) -> Self {
        Self(id.into())
    }

    /// A category from a `&'static str` id, in a `const` context.
    pub const fn from_static(id: &'static str) -> Self {
        Self(HipStr::from_static(id))
    }

    /// The category id as a string slice.
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
