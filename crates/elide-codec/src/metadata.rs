//! [`MetadataFields`]: the fields a metadata stream surfaces, read and redacted
//! in place.

use elide_core::Result;
use elide_core::modality::Chunk;
use elide_core::modality::metadata::{
    Metadata, MetadataData, MetadataLocation, MetadataReplacement,
};
use elide_core::redaction::Redactions;

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
    /// holding only that part of its value. `None` when no field has the key
    /// or the span is not on the value's character boundaries.
    #[must_use]
    pub fn read_at(&self, location: &MetadataLocation) -> Option<MetadataData> {
        let field = self.field(&location.key)?;
        match &location.span {
            Some(span) => Some(MetadataData::new(
                field.key.clone(),
                field.as_str().get(span.clone())?.to_owned(),
                field.label.clone(),
            )),
            None => Some(field.clone()),
        }
    }

    /// Apply `redactions` to the fields' values, returning each edited field's
    /// key and new value, a blank value for a field now removed.
    ///
    /// Applied right to left, so an edit's length delta does not move an
    /// earlier span in the same value. A redaction for a key no field has is
    /// skipped.
    ///
    /// # Errors
    ///
    /// A redaction error if a span falls mid-character.
    pub fn redact(
        &mut self,
        mut redactions: Redactions<Metadata>,
    ) -> Result<Vec<(String, String)>> {
        redactions.sort_by_position();
        let mut edited: Vec<String> = Vec::new();
        for (location, replacement) in redactions.into_iter().rev() {
            let MetadataReplacement::Value(replacement) = replacement else {
                continue;
            };
            let Some(field) = self
                .fields
                .iter_mut()
                .find(|field| field.key == location.key)
            else {
                continue;
            };
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
        let edits = edited
            .into_iter()
            .filter_map(|key| {
                let value = self.field(&key)?.as_str().trim().to_owned();
                Some((key, value))
            })
            .collect();
        self.fields
            .retain(|field| !field.as_str().trim().is_empty());
        Ok(edits)
    }

    /// The field keyed `key`.
    fn field(&self, key: &str) -> Option<&MetadataData> {
        self.fields.iter().find(|field| field.key == key)
    }
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

    fn batch(pairs: Vec<(MetadataLocation, TextReplacement)>) -> Redactions<Metadata> {
        let mut redactions = Redactions::new();
        for (location, replacement) in pairs {
            redactions.push(location, replacement.into());
        }
        redactions
    }

    fn part(start: usize, end: usize) -> MetadataLocation {
        MetadataLocation::new("description").with_span(start..end)
    }

    #[test]
    fn a_span_reads_its_part_of_the_value() {
        let read = fields().read_at(&part(9, 17)).unwrap();
        assert_eq!(read.value, "Jane Roe");
        assert!(fields().read_at(&part(0, 99)).is_none());
        assert!(
            fields()
                .read_at(&MetadataLocation::new("keywords"))
                .is_none()
        );
    }

    #[test]
    fn spans_of_a_value_are_redacted_in_place() {
        let mut fields = fields();
        let edits = fields
            .redact(batch(vec![
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
            "Photo of [NAME] in "
        );
    }

    #[test]
    fn a_field_left_blank_is_removed() {
        let mut fields = fields();
        let edits = fields
            .redact(batch(vec![(
                MetadataLocation::new("title"),
                TextReplacement::Removed,
            )]))
            .unwrap();
        assert_eq!(edits, [("title".to_owned(), String::new())]);
        assert!(fields.read_at(&MetadataLocation::new("title")).is_none());
        assert_eq!(fields.chunks().len(), 1);
    }

    #[test]
    fn unchanged_and_unknown_fields_edit_nothing() {
        let mut fields = fields();
        let mut redactions = Redactions::new();
        redactions.push(
            MetadataLocation::new("title"),
            MetadataReplacement::Unchanged,
        );
        redactions.push(
            MetadataLocation::new("keywords"),
            TextReplacement::Removed.into(),
        );
        assert!(fields.redact(redactions).unwrap().is_empty());
    }
}
