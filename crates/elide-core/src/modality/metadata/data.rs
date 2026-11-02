//! [`MetadataData`]: one metadata field's key, value, and meaning.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::LabelRef;
use crate::modality::ModalityData;

/// One metadata field as a recognizer sees it: its key, its value, and what it
/// is.
///
/// A metadata chunk is a single field. The [`key`](Self::key) identifies it
/// (`"GPSLatitude"`, `"modified"`), the [`value`](Self::value) is its content
/// rendered as text (a coordinate, a device name, a timestamp), and the
/// [`label`](Self::label) is what the field is by the format's own schema (a
/// location, a person, a time), stamped by the codec that surfaced it. A field
/// whose meaning the schema does not fix (a free-text comment) carries none.
/// A recognizer turns a label into an entity, the same way for every format
/// (`elide-detection`'s `FieldRecognizer`).
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
    /// What the field is by its format's schema, or `None` when the schema
    /// does not fix it.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub label: Option<LabelRef>,
}

impl MetadataData {
    /// A field with `key` and `value`.
    pub fn new(key: impl Into<HipStr<'static>>, value: impl Into<HipStr<'static>>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            label: None,
        }
    }

    /// This field, labelled by what its format's schema says it is.
    #[must_use]
    pub fn with_label(mut self, label: LabelRef) -> Self {
        self.label = Some(label);
        self
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

    /// What the field is by its format's schema, if the schema fixes it.
    #[must_use]
    pub fn label(&self) -> Option<&LabelRef> {
        self.label.as_ref()
    }
}

impl ModalityData for MetadataData {}
