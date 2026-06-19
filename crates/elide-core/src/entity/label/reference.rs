//! [`LabelRef`] lightweight reference.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Lightweight reference to a [`Label`], carrying only its name.
///
/// This is what detections and entities hold: cloning is cheap (short
/// names inline into the [`HipStr`]), and the full [`Label`], with its
/// description, is resolved on demand from a [`LabelCatalog`].
///
/// [`Label`]: crate::entity::Label
/// [`LabelCatalog`]: crate::entity::LabelCatalog
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct LabelRef(HipStr<'static>);

impl LabelRef {
    /// Reference a label by name.
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self(name.into())
    }

    /// Referenced label's name.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
