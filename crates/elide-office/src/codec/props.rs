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

use bytes::Bytes;
use elide_codec::metadata::MetadataFields;
use elide_codec::{FlaggedItem, FormatId, LocalId, Stream, StreamMut};
use elide_core::Result;
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use crate::opc::{PartPath, PartReplacement, props};

/// Stable [`FormatId`] for an OOXML property part's metadata stream.
pub const FORMAT_ID: FormatId = FormatId::new("elide.office.docprops");

/// The document-property parts an OOXML package surfaces as metadata.
const DOCPROPS_PARTS: &[&str] = &[
    "docProps/core.xml",
    "docProps/app.xml",
    "docProps/custom.xml",
];

/// A package's document-property parts, each a metadata stream keyed by its
/// zip entry path. Shared by every OOXML document (docx, pptx, xlsx).
#[derive(Debug, Default)]
pub(crate) struct DocProps(Vec<(LocalId, DocPropsStream)>);

impl DocProps {
    /// The property parts a package carries, read through its `part_bytes`
    /// accessor. A part the package lacks is skipped.
    pub(crate) fn read(mut part_bytes: impl FnMut(&str) -> Option<Bytes>) -> Self {
        Self(
            DOCPROPS_PARTS
                .iter()
                .filter_map(|path| {
                    Some((LocalId::new(*path), DocPropsStream::new(part_bytes(path)?)))
                })
                .collect(),
        )
    }

    /// Lend each property stream, with its id.
    pub(crate) fn streams(&mut self) -> impl Iterator<Item = (LocalId, StreamMut<'_>)> {
        self.0
            .iter_mut()
            .map(|(id, stream)| (id.clone(), StreamMut::new::<Metadata, _>(stream)))
    }

    /// Each property part's (possibly redacted) XML, as a replacement for the
    /// package part it was read from.
    pub(crate) fn replacements(&self) -> Vec<PartReplacement> {
        self.0
            .iter()
            .map(|(id, stream)| {
                PartReplacement::new(PartPath::from(id.as_str()), stream.xml.to_vec())
            })
            .collect()
    }
}

/// Stream for an OOXML property part: yields its labelled fields and rewrites
/// the property XML as each batch of redactions edits them.
#[derive(Debug)]
pub(crate) struct DocPropsStream {
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
    async fn read_at(&self, location: &MetadataLocation) -> Result<Option<MetadataData>> {
        Ok(self.fields.read_at(location))
    }
}

#[::async_trait::async_trait]
impl DataWriter<Metadata> for DocPropsStream {
    async fn write_at(&mut self, redactions: Redactions<Metadata>) -> Result<()> {
        let edits = self.fields.redact(redactions)?;
        let edits: Vec<(&str, &str)> = edits
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        self.xml = props::rewrite(&self.xml, &edits);
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
        let mut redactions = Redactions::new();
        redactions.push(
            MetadataLocation::new("creator"),
            TextReplacement::substituted("[REDACTED]").into(),
        );
        redactions.push(
            MetadataLocation::new("title"),
            TextReplacement::Removed.into(),
        );
        stream.write_at(redactions).await.unwrap();
        let xml = stream.xml.clone();
        let fields = props::fields(&xml);
        assert!(
            fields
                .iter()
                .any(|field| field.key == "creator" && field.value == "[REDACTED]")
        );
        assert!(!fields.iter().any(|field| field.key == "title"));
    }
}
