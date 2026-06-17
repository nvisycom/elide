//! Entity labels: the taxonomy of what an entity *is*.
//!
//! A [`Label`] is a named kind of sensitive information ("PHONE_NUMBER")
//! with an optional human description. Detections and entities don't
//! carry the full label — they carry a lightweight [`LabelRef`] (the
//! name only), and the descriptions live once in a [`LabelCatalog`].
//! This keeps the per-detection footprint small while still letting a
//! consumer resolve a reference back to its full definition.

mod catalog;
mod reference;

pub use self::catalog::LabelCatalog;
pub use self::reference::LabelRef;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A kind of sensitive information, with an optional description.
///
/// Names are conventionally `SCREAMING_SNAKE_CASE` (`"PHONE_NUMBER"`),
/// matching Presidio, but this is convention, not enforcement. The
/// taxonomy is open: a [`Label`] can be minted for any name a
/// recognizer or configuration needs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Label {
    name: HipStr<'static>,
    description: Option<HipStr<'static>>,
}

impl Label {
    /// A label with just a name and no description.
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            description: None,
        }
    }

    /// A label with a name and a human-readable description.
    pub fn described(
        name: impl Into<HipStr<'static>>,
        description: impl Into<HipStr<'static>>,
    ) -> Self {
        Self {
            name: name.into(),
            description: Some(description.into()),
        }
    }

    /// The label's name.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The label's description, if any.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// A lightweight reference to this label, by name.
    pub fn as_ref(&self) -> LabelRef {
        LabelRef::new(self.name.clone())
    }

    /// The label's name as an owned string (for catalog keying).
    fn name_owned(&self) -> HipStr<'static> {
        self.name.clone()
    }
}

impl From<&Label> for LabelRef {
    fn from(label: &Label) -> Self {
        label.as_ref()
    }
}
