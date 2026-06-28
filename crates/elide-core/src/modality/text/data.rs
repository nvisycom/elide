//! [`TextData`]: the text payload a recognizer inspects.

use hipstr::HipStr;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityData;

/// Run of text.
///
/// Either the payload a text recognizer inspects, or the value sliced out
/// at an entity's location for an operator.
///
/// Held as a [`HipStr`] so short values inline and longer ones share a
/// refcounted buffer, making cheap clones when one payload is passed to
/// several recognizers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct TextData {
    /// Text content.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub text: HipStr<'static>,
}

impl TextData {
    /// Wrap a string as text data.
    pub fn new(text: impl Into<HipStr<'static>>) -> Self {
        Self { text: text.into() }
    }

    /// Text as a string slice.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }
}

impl ModalityData for TextData {}
