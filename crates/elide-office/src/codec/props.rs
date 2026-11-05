//! [`DocPropsStream`]: an OOXML document-property part (`docProps/core.xml`,
//! `docProps/app.xml`) as the [`Metadata`] modality, shared by docx, pptx, and
//! xlsx.
//!
//! When metadata is treated as content, every OOXML package surfaces its
//! property parts as metadata stream parts beside its body. Each stream yields
//! one chunk per property field, labelled with what it is (an author, a
//! timestamp, a company), applies each field's redaction via [`write_at`] (a
//! removal clears the field, an overwrite writes its new text), and re-encodes
//! the property XML, which the owning package's recombiner writes back.
//!
//! [`Metadata`]: elide_core::modality::metadata::Metadata
//! [`write_at`]: elide_core::modality::DataWriter::write_at

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{DocumentPart, ErasedStream, FlaggedItem, FormatId, LocalId, Stream};
use elide_core::Result;
use elide_core::modality::metadata::{
    Metadata, MetadataData, MetadataLocation, MetadataReplacement,
};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use crate::opc::props;

/// Stable [`FormatId`] for an OOXML property part's metadata stream.
pub const FORMAT_ID: FormatId = FormatId::new("elide.office.docprops");

/// The document-property parts an OOXML package surfaces as metadata.
const DOCPROPS_PARTS: &[&str] = &["docProps/core.xml", "docProps/app.xml"];

/// The document-property parts a package carries, via its `part_bytes`
/// accessor, each as a metadata stream part keyed by its zip entry path.
/// Shared by every OOXML loader (docx, pptx, xlsx). A part the package lacks
/// is skipped.
pub(crate) fn doc_props_parts(
    mut part_bytes: impl FnMut(&str) -> Option<Bytes>,
) -> Vec<DocumentPart> {
    DOCPROPS_PARTS
        .iter()
        .filter_map(|path| {
            let stream: Box<dyn Stream<Metadata>> =
                Box::new(DocPropsStream::new(part_bytes(path)?));
            Some(DocumentPart::Stream {
                id: LocalId::new(*path),
                handle: ErasedStream::new(FORMAT_ID, stream),
            })
        })
        .collect()
}

/// Stream for an OOXML property part: yields its labelled fields, records the
/// edits picked for them, and re-encodes the property XML with those applied.
#[derive(Debug)]
pub(crate) struct DocPropsStream {
    /// The property part's XML bytes.
    xml: Bytes,
    /// Every field, parsed once at decode and kept in document order: the chunks
    /// a caller sees and the set [`read_at`](DataReader::read_at) looks one up in
    /// without re-parsing the XML.
    fields: Vec<MetadataData>,
    /// Field keys (local names) picked for removal, applied on `encode`.
    removed: Vec<String>,
    /// Field keys picked for an overwrite, with their new text, applied on
    /// `encode`.
    replaced: Vec<(String, String)>,
}

impl DocPropsStream {
    /// Wrap a property part's XML, reading its fields once.
    pub(crate) fn new(xml: Bytes) -> Self {
        let fields: Vec<MetadataData> = props::fields(&xml)
            .into_iter()
            .filter_map(|(key, value)| {
                let label = props::label(&key)?;
                Some(MetadataData::new(key, value, label))
            })
            .collect();
        Self {
            xml,
            fields,
            removed: Vec::new(),
            replaced: Vec::new(),
        }
    }
}

impl Stream<Metadata> for DocPropsStream {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        let removed: Vec<&str> = self.removed.iter().map(String::as_str).collect();
        let replaced: Vec<(&str, &str)> = self
            .replaced
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        Ok(ContentData::new(props::rewrite(
            &self.xml, &removed, &replaced,
        )))
    }

    fn chunks(&self) -> Result<Vec<Chunk<Metadata>>> {
        Ok(self
            .fields
            .iter()
            .map(|field| Chunk::new(MetadataLocation::new(field.key.clone()), field.clone()))
            .collect())
    }

    fn flagged(&self) -> Result<Vec<FlaggedItem<Metadata>>> {
        Ok(self.fields.iter().map(FlaggedItem::field).collect())
    }
}

#[::async_trait::async_trait]
impl DataReader<Metadata> for DocPropsStream {
    async fn read_at(&self, location: &MetadataLocation) -> Result<Option<MetadataData>> {
        let key = location.key.as_str();
        Ok(self.fields.iter().find(|field| field.key == key).cloned())
    }
}

#[::async_trait::async_trait]
impl DataWriter<Metadata> for DocPropsStream {
    async fn write_at(&mut self, redactions: Redactions<Metadata>) -> Result<()> {
        for (location, replacement) in redactions.into_iter() {
            let key = location.key.as_str().to_owned();
            match replacement {
                MetadataReplacement::Replace(value) => self.replaced.push((key, value)),
                MetadataReplacement::Unchanged => {}
                // Removal, and any treatment added later, clears the field:
                // nothing leaves it intact.
                _ => self.removed.push(key),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::{LabelRef, builtins};

    use super::*;

    const CORE: &[u8] = br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jane Roe</dc:creator><dc:title>Quarterly plan</dc:title><cp:keywords>acquisition</cp:keywords></cp:coreProperties>"#;

    #[test]
    fn each_field_carries_what_its_schema_says_it_is() {
        let stream = DocPropsStream::new(Bytes::from_static(CORE));
        let label = |key: &str| {
            stream
                .fields
                .iter()
                .find(|field| field.key() == key)
                .map(|field| field.label().clone())
        };
        assert_eq!(
            label("creator"),
            Some(LabelRef::from(&*builtins::PERSON_NAME))
        );
        assert_eq!(
            label("title"),
            Some(LabelRef::from(&*builtins::COMMUNICATIONS_CONTENT))
        );
        // A field the schema does not label is not surfaced at all.
        assert_eq!(label("keywords"), None);
    }

    #[tokio::test]
    async fn write_at_removes_and_overwrites_fields() {
        let mut stream = DocPropsStream::new(Bytes::from_static(CORE));
        let mut redactions = Redactions::new();
        redactions.push(
            MetadataLocation::new("creator"),
            MetadataReplacement::Replace("[REDACTED]".to_owned()),
        );
        redactions.push(MetadataLocation::new("title"), MetadataReplacement::Removed);
        stream.write_at(redactions).await.unwrap();
        let xml = stream.encode().unwrap().into_bytes();
        let fields = props::fields(&xml);
        assert!(fields.contains(&("creator".to_owned(), "[REDACTED]".to_owned())));
        assert!(!fields.iter().any(|(key, _)| key == "title"));
    }
}
