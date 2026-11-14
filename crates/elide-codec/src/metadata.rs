//! The fields a metadata stream surfaces, read and redacted in place.

use elide_core::modality::Chunk;
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use crate::FlaggedItem;
use crate::string::RedactRange;

/// The fields a metadata stream surfaces, in document order.
///
/// A metadata stream holds its fields here and delegates to it: the chunks and
/// flagged items it reports, the value a location reads, and the edits a batch
/// of redactions makes. It works like a cell store: each redaction applies to
/// the field's value, or the part of it a location's span narrows to, and the
/// stream writes back what [`redact`](Self::redact) returns. A field left
/// blank is removed, from the format and from here.
#[derive(Debug, Clone, Default)]
pub struct MetadataFields {
    fields: Vec<MetadataData>,
}

impl MetadataFields {
    /// The fields `fields`, in document order.
    #[must_use]
    pub fn new(fields: Vec<MetadataData>) -> Self {
        Self { fields }
    }

    /// One chunk per field.
    #[must_use]
    pub fn chunks(&self) -> Vec<Chunk<Metadata>> {
        self.fields
            .iter()
            .map(|field| Chunk::new(MetadataLocation::new(field.key.clone()), field.clone()))
            .collect()
    }

    /// One flagged item per field, labelled as its format labels it.
    #[must_use]
    pub fn flagged(&self) -> Vec<FlaggedItem<Metadata>> {
        self.fields.iter().map(FlaggedItem::field).collect()
    }

    /// What `location` addresses: the whole field, or for a span, the field
    /// holding only that part of its value.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if no field has the key, or the span is outside the
    /// value or splits a character.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn read_at(&self, location: &MetadataLocation) -> Result<MetadataData> {
        let field = self.field(&location.key)?;
        let Some(span) = &location.span else {
            return Ok(field.clone());
        };
        let value = field.as_str().get(span.clone()).ok_or_else(|| {
            Error::malformed(format!(
                "span {}..{} is outside the field's value or splits a character",
                span.start, span.end
            ))
        })?;
        Ok(MetadataData::new(
            field.key.clone(),
            value.to_owned(),
            field.label.clone(),
        ))
    }

    /// Apply `redactions` to the fields' values, returning each edited field.
    ///
    /// Each edited field comes back as its key and new value, trimmed, and
    /// blank for a field now removed. The fields keep the same trimmed value,
    /// so a later read matches what the format wrote.
    ///
    /// Applied right to left, so an edit's length delta does not move an
    /// earlier span in the same value.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if no field has a redaction's key, or its span is
    /// outside the value or splits a character. Nothing is skipped.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn redact(
        &mut self,
        mut redactions: Redactions<Metadata>,
    ) -> Result<Vec<(String, String)>> {
        redactions.sort_by_position();
        let mut edited: Vec<String> = Vec::new();
        for (location, replacement) in redactions.into_iter().rev() {
            let field = self
                .fields
                .iter_mut()
                .find(|field| field.key == location.key)
                .ok_or_else(|| unknown_field(&location.key))?;
            let mut value = field.as_str().to_owned();
            let span = location.span.unwrap_or(0..value.len());
            value.redact_range(replacement.value().unwrap_or_default(), span)?;
            field.value = value.into();
            if !edited
                .iter()
                .any(|key| key.as_str() == location.key.as_str())
            {
                edited.push(location.key.as_str().to_owned());
            }
        }
        // Trim once every span of a field is applied: trimming between edits
        // would shift the spans still to come.
        let mut edits = Vec::with_capacity(edited.len());
        for key in edited {
            let Some(field) = self.fields.iter_mut().find(|field| field.key == key) else {
                continue;
            };
            let value = field.as_str().trim().to_owned();
            field.value = value.clone().into();
            edits.push((key, value));
        }
        self.fields
            .retain(|field| !field.as_str().trim().is_empty());
        Ok(edits)
    }

    /// The field keyed `key`.
    fn field(&self, key: &str) -> Result<&MetadataData> {
        self.fields
            .iter()
            .find(|field| field.key == key)
            .ok_or_else(|| unknown_field(key))
    }
}

/// The error for a location naming a field the stream doesn't have. The key
/// is a format's field name, not document content.
fn unknown_field(key: &str) -> Error {
    Error::malformed(format!("no metadata field `{key}`"))
}

#[cfg(test)]
mod tests {
    use elide_core::entity::LabelRef;
    use elide_core::modality::text::TextReplacement;

    use super::*;

    fn fields() -> MetadataFields {
        MetadataFields::new(vec![
            MetadataData::new(
                "description",
                "Photo of Jane Roe in Kyiv",
                LabelRef::new("x"),
            ),
            MetadataData::new("title", "Plan", LabelRef::new("x")),
        ])
    }

    fn part(start: usize, end: usize) -> MetadataLocation {
        MetadataLocation::new("description").with_span(start..end)
    }

    #[test]
    fn a_span_reads_its_part_of_the_value() {
        let read = fields().read_at(&part(9, 17)).unwrap();
        assert_eq!(read.value, "Jane Roe");
        assert!(fields().read_at(&part(0, 99)).is_err());
        assert!(
            fields()
                .read_at(&MetadataLocation::new("keywords"))
                .is_err()
        );
    }

    #[test]
    fn spans_of_a_value_are_redacted_in_place() {
        let mut fields = fields();
        let edits = fields
            .redact(Redactions::from([
                (part(9, 17), TextReplacement::substituted("[NAME]")),
                (part(21, 25), TextReplacement::Removed),
            ]))
            .unwrap();
        assert_eq!(
            edits,
            [("description".to_owned(), "Photo of [NAME] in".to_owned())]
        );
        assert_eq!(
            fields
                .read_at(&MetadataLocation::new("description"))
                .unwrap()
                .value,
            "Photo of [NAME] in"
        );
    }

    #[test]
    fn a_field_left_blank_is_removed() {
        let mut fields = fields();
        let edits = fields
            .redact(Redactions::from([(
                MetadataLocation::new("title"),
                TextReplacement::Removed,
            )]))
            .unwrap();
        assert_eq!(edits, [("title".to_owned(), String::new())]);
        assert!(fields.read_at(&MetadataLocation::new("title")).is_err());
        assert_eq!(fields.chunks().len(), 1);
    }

    #[test]
    fn an_unknown_field_is_an_error() {
        let mut fields = fields();
        let redactions =
            Redactions::from([(MetadataLocation::new("keywords"), TextReplacement::Removed)]);
        assert!(fields.redact(redactions).is_err());
    }
}
