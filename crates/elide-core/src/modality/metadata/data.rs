//! [`MetadataData`]: one metadata field's key and value.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityData;

/// One metadata field as a recognizer sees it: its key and its value.
///
/// A metadata chunk is a single field. The [`key`](Self::key) identifies it
/// (`"GPSLatitude"`, `"modified"`) and the [`value`](Self::value) is its
/// content rendered as text (a coordinate, a device name, a timestamp). A
/// recognizer reads both — the key to decide *what kind* of field it is (and
/// whether it is sensitive), the value the way a text recognizer reads a span.
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
}

impl MetadataData {
    /// A field with `key` and `value`.
    pub fn new(key: impl Into<HipStr<'static>>, value: impl Into<HipStr<'static>>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    /// The field key.
    #[must_use]
    pub fn key(&self) -> &str {
        self.key.as_str()
    }

    /// The field value.
    #[must_use]
    pub fn value(&self) -> &str {
        self.value.as_str()
    }
}

impl ModalityData for MetadataData {}
