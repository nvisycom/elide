//! [`DocPropsStream`]: an OOXML document-property part (`docProps/core.xml`,
//! `docProps/app.xml`, `docProps/custom.xml`) as the [`Metadata`] modality,
//! shared by docx, pptx, and xlsx.
//!
//! When metadata is treated as content, every OOXML package surfaces its
//! property parts as metadata streams beside its body. Each stream yields
//! one chunk per property field, labelled with what it is (an author, a
//! timestamp, a company), applies each redaction to the field's value via
//! [`write_at`] (the whole value, or the part a span narrows to), and rewrites
//! the property XML with the edited fields, a field left blank removed, which
//! the owning package's document writes back.
//!
//! [`Metadata`]: elide_core::modality::metadata::Metadata
//! [`write_at`]: elide_core::modality::DataWriter::write_at

use std::collections::BTreeMap;

use bytes::Bytes;
use elide_codec::metadata::MetadataFields;
use elide_codec::{FlaggedItem, FormatId, LocalId, Stream, StreamMut};
use elide_core::Result;
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::{HiddenContentPolicy, Redactions};

use crate::codec::local_id;
use crate::opc::{PartPath, PartReplacement, props};

/// Stable [`FormatId`] for an OOXML property part's metadata stream.
pub(crate) const FORMAT_ID: FormatId = FormatId::new("elide.office.docprops");

/// A package's document-property parts, each a metadata stream keyed by the
/// part it was read from. Shared by every OOXML document (docx, pptx, xlsx).
#[derive(Debug, Default)]
pub(crate) struct DocProps {
    /// Each property part the package carries, with its stream, in
    /// [`props::SURFACED_PARTS`] order.
    parts: Vec<(PartPath, DocPropsStream)>,
}

impl DocProps {
    /// The property parts a package carries when `policy` treats metadata as
    /// content (scanned or flagged), read through its `part_bytes` accessor;
    /// none when it keeps only the essential metadata.
    pub(crate) fn under(
        policy: HiddenContentPolicy,
        part_bytes: impl FnMut(&str) -> Option<Bytes>,
    ) -> Self {
        if policy.metadata.treats_as_content() {
            Self::read(part_bytes)
        } else {
            Self::default()
        }
    }

    /// The property parts a package carries, read through its `part_bytes`
    /// accessor. A part the package lacks is skipped.
    fn read(mut part_bytes: impl FnMut(&str) -> Option<Bytes>) -> Self {
        let parts = props::SURFACED_PARTS
            .iter()
            .filter_map(|path| {
                Some((
                    PartPath::from(*path),
                    DocPropsStream::new(part_bytes(path)?),
                ))
            })
            .collect();
        Self { parts }
    }

    /// Lend each property stream, with its id: the path of its part.
    pub(crate) fn streams(&mut self) -> impl Iterator<Item = (LocalId, StreamMut<'_>)> {
        self.parts
            .iter_mut()
            .map(|(path, stream)| (local_id(path), StreamMut::new::<Metadata, _>(stream)))
    }

    /// Each property part's (possibly redacted) XML, as a replacement for the
    /// package part it was read from.
    pub(crate) fn replacements(&self) -> Vec<PartReplacement> {
        self.parts
            .iter()
            .map(|(path, stream)| PartReplacement::new(path.clone(), stream.xml.clone()))
            .collect()
    }
}

/// Stream for an OOXML property part: yields its labelled fields and rewrites
/// the property XML as each batch of redactions edits them.
#[derive(Debug)]
pub(crate) struct DocPropsStream {
    /// The property part's XML bytes as read, which every rewrite starts
    /// from: a custom property is keyed by its position here, and one
    /// dropped by a rewrite would shift those after it.
    original: Bytes,
    /// Each field's value as edited so far, by key, the latest batch's
    /// winning.
    edits: BTreeMap<String, String>,
    /// The property part's XML bytes, as edited so far.
    xml: Bytes,
    /// Every field, parsed once at decode and kept in document order.
    fields: MetadataFields,
}

impl DocPropsStream {
    /// Wrap a property part's XML, reading its fields once.
    pub(crate) fn new(xml: Bytes) -> Self {
        let fields = props::fields(&xml)
            .into_iter()
            .map(|field| MetadataData::new(field.key, field.value, field.label))
            .collect();
        Self {
            original: xml.clone(),
            edits: BTreeMap::new(),
            xml,
            fields: MetadataFields::new(fields),
        }
    }
}

impl Stream<Metadata> for DocPropsStream {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Metadata>>> {
        Ok(self.fields.chunks())
    }

    fn flagged(&self) -> Result<Vec<FlaggedItem<Metadata>>> {
        Ok(self.fields.flagged())
    }
}

#[::async_trait::async_trait]
impl DataReader<Metadata> for DocPropsStream {
    async fn read_at(&self, location: &MetadataLocation) -> Result<MetadataData> {
        self.fields.read_at(location)
    }
}

#[::async_trait::async_trait]
impl DataWriter<Metadata> for DocPropsStream {
    async fn write_at(&mut self, redactions: Redactions<Metadata>) -> Result<()> {
        self.edits.extend(self.fields.redact(redactions)?);
        let edits: Vec<(&str, &str)> = self
            .edits
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        self.xml = props::rewrite(&self.original, &edits);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::{LabelRef, builtins};
    use elide_core::modality::text::TextReplacement;

    use super::*;

    const CORE: &[u8] = br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jane Roe</dc:creator><dc:title>Quarterly plan</dc:title><cp:keywords>acquisition</cp:keywords></cp:coreProperties>"#;

    #[test]
    fn each_field_carries_what_its_schema_says_it_is() {
        let stream = DocPropsStream::new(Bytes::from_static(CORE));
        let label = |key: &str| {
            stream
                .chunks()
                .unwrap()
                .into_iter()
                .find(|chunk| chunk.data.key == key)
                .map(|chunk| chunk.data.label.clone())
        };
        assert_eq!(
            label("creator"),
            Some(LabelRef::from(&*builtins::PERSON_NAME))
        );
        assert_eq!(
            label("title"),
            Some(LabelRef::from(&*builtins::COMMUNICATIONS_CONTENT))
        );
        assert_eq!(
            label("keywords"),
            Some(LabelRef::from(&*builtins::COMMUNICATIONS_CONTENT))
        );
    }

    #[tokio::test]
    async fn write_at_removes_and_overwrites_fields() {
        let mut stream = DocPropsStream::new(Bytes::from_static(CORE));
        stream
            .write_at(Redactions::from([
                (
                    MetadataLocation::new("creator"),
                    TextReplacement::substituted("[REDACTED]"),
                ),
                (MetadataLocation::new("title"), TextReplacement::Removed),
            ]))
            .await
            .unwrap();
        let xml = stream.xml.clone();
        let fields = props::fields(&xml);
        assert!(
            fields
                .iter()
                .any(|field| field.key == "creator" && field.value == "[REDACTED]")
        );
        assert!(!fields.iter().any(|field| field.key == "title"));
    }

    /// A custom property is keyed by its position when the part is read; one
    /// dropped by a batch must not shift the property a later batch names.
    #[tokio::test]
    async fn a_later_batch_edits_the_property_it_was_read_as() {
        const CUSTOM: &[u8] = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Acme</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Owner"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="4" name="City"><vt:lpwstr>Kyiv</vt:lpwstr></property></Properties>"#;
        let mut stream = DocPropsStream::new(Bytes::from_static(CUSTOM));
        stream
            .write_at(Redactions::from([(
                MetadataLocation::new("property/0"),
                TextReplacement::Removed,
            )]))
            .await
            .unwrap();
        stream
            .write_at(Redactions::from([(
                MetadataLocation::new("property/1"),
                TextReplacement::substituted("[NAME]"),
            )]))
            .await
            .unwrap();
        let xml = stream.xml.clone();
        let text = std::str::from_utf8(&xml).unwrap();
        assert!(
            !text.contains("Acme") && !text.contains("Jane Roe"),
            "{text}"
        );
        let values: Vec<String> = props::fields(&xml)
            .into_iter()
            .map(|field| field.value)
            .collect();
        assert_eq!(values, ["Owner", "[NAME]", "City", "Kyiv"]);
    }
}
