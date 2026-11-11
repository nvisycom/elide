//! [`MetadataData`]: one metadata field's key, value, and meaning.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::LabelRef;
use crate::modality::ModalityData;

/// One metadata field: its key, its value, and what it is.
///
/// A metadata chunk is a single field. The [`key`](Self::key) identifies it
/// (`"GPSLatitude"`, `"modified"`), the [`value`](Self::value) is its content
/// rendered as text (a coordinate, a device name, a timestamp), and the
/// [`label`](Self::label) is what the field is by the format's own schema (a
/// location, a person, a time), stamped by the codec that surfaced it. The
/// label flags the field whole; the text recognizers read the value for what it
/// says.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MetadataData {
    /// The field's key.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub key: HipStr<'static>,
    /// The field's value as text.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub value: HipStr<'static>,
    /// What the field is by its format's schema.
    pub label: LabelRef,
}

impl MetadataData {
    /// A field with `key` and `value` that its format's schema labels `label`.
    pub fn new(
        key: impl Into<HipStr<'static>>,
        value: impl Into<HipStr<'static>>,
        label: LabelRef,
    ) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            label,
        }
    }

    /// The field's value as a string slice, what an operator reads, as
    /// [`TextData::as_str`] does for text.
    ///
    /// [`TextData::as_str`]: crate::modality::text::TextData::as_str
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

impl ModalityData for MetadataData {}
