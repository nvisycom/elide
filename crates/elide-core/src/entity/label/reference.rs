//! [`LabelRef`] lightweight reference.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Lightweight reference to a [`Label`], carrying only its id.
///
/// This is what detections and entities hold: cloning is cheap (short
/// ids inline into the [`HipStr`]), and the full [`Label`], with its
/// localized names and descriptions, is resolved on demand from a
/// [`LabelCatalog`].
///
/// [`Label`]: crate::entity::Label
/// [`LabelCatalog`]: crate::entity::LabelCatalog
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct LabelRef(#[cfg_attr(feature = "schema", schemars(with = "String"))] HipStr<'static>);

impl LabelRef {
    /// Reference a label by id.
    ///
    /// Accepts any id source — a `&'static str`, an owned `String`, a
    /// [`HipStr`]. For a `&'static str` literal known at compile time, prefer
    /// [`from_static`](Self::from_static), which is `const`.
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self(name.into())
    }

    /// Reference a label by a static id, in a `const` context.
    ///
    /// The `const` counterpart to [`new`](Self::new) for a `&'static str`
    /// literal, so a `LabelRef` can be a `const`/`static` item with no runtime
    /// construction:
    ///
    /// ```
    /// # use elide_core::entity::LabelRef;
    /// const EMAIL: LabelRef = LabelRef::from_static("EMAIL_ADDRESS");
    /// assert_eq!(EMAIL.as_str(), "EMAIL_ADDRESS");
    /// ```
    pub const fn from_static(name: &'static str) -> Self {
        Self(HipStr::from_static(name))
    }

    /// Referenced label's name.
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
