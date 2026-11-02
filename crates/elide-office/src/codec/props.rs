//! Stream and loader for an OOXML document-property part, a `docProps/*`
//! sub-part shared by docx, pptx, and xlsx.
//!
//! Every OOXML package exposes its property parts (`docProps/core.xml`,
//! `docProps/app.xml`) as nested sub-parts, the same way it exposes an embedded
//! image. This handler decodes such a part as the [`Metadata`] modality: it
//! streams each property field as a chunk, labelled with what it is (an
//! author, a timestamp, a company) so the format-neutral field recognizer
//! (`elide-detection`'s `FieldRecognizer`) can turn it into an entity, records the fields picked for removal via [`write_at`],
//! and re-encodes the property XML with exactly those fields cleared. The owning
//! package handler then folds the edited part back in.
//!
//! [`Metadata`]: elide_core::modality::metadata::Metadata
//! [`write_at`]: elide_core::modality::DataWriter::write_at

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{FormatId, Loader, Stream};
use elide_core::Result;
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use crate::opc::props;

/// Stable [`FormatId`] for an OOXML property sub-part.
pub const FORMAT_ID: FormatId = FormatId::new("elide.office.docprops");

/// The pseudo-extension a `docProps/*` sub-part is decoded with; it resolves this
/// format in the registry (the fold looks a part up by its hint as an extension).
pub const PROPS_HINT: &str = "x-elide-docprops";

/// The document-property parts an OOXML package surfaces as metadata sub-parts.
const DOCPROPS_PARTS: &[&str] = &["docProps/core.xml", "docProps/app.xml"];

/// Read the document-property parts a package carries, as `(zip entry, bytes)`,
/// via its `part_bytes` accessor. Shared by every OOXML loader (docx, pptx,
/// xlsx) so they cache the same set at decode. A part the package lacks is
/// skipped.
pub(crate) fn read_doc_props(
    mut part_bytes: impl FnMut(&str) -> Option<Bytes>,
) -> Vec<(String, Bytes)> {
    DOCPROPS_PARTS
        .iter()
        .filter_map(|path| part_bytes(path).map(|bytes| ((*path).to_owned(), bytes)))
        .collect()
}

/// Stream for an OOXML property part: streams fields, clears chosen ones,
/// re-encodes the property XML.
#[derive(Debug)]
pub(crate) struct DocPropsHandler {
    /// The property part's XML bytes.
    xml: Bytes,
    /// Every field, parsed once at decode and kept in document order: the chunks
    /// a caller sees and the set [`read_at`](DataReader::read_at) looks one up in
    /// without re-parsing the XML.
    fields: Vec<MetadataData>,
    /// Field keys (local names) picked for removal, applied on `encode`.
    removed: Vec<String>,
}

impl DocPropsHandler {
    /// Wrap a property part's XML, reading its fields once.
    pub(crate) fn new(xml: Bytes) -> Self {
        let fields: Vec<MetadataData> = props::fields(&xml)
            .into_iter()
            .map(|(key, value)| {
                let label = props::label(&key);
                let field = MetadataData::new(key, value);
                match label {
                    Some(label) => field.with_label(label),
                    None => field,
                }
            })
            .collect();
        Self {
            xml,
            fields,
            removed: Vec::new(),
        }
    }
}

impl Stream<Metadata> for DocPropsHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        let keys: Vec<&str> = self.removed.iter().map(String::as_str).collect();
        Ok(ContentData::new(props::strip(&self.xml, &keys)))
    }

    fn chunks(&self) -> Result<Vec<Chunk<Metadata>>> {
        Ok(self
            .fields
            .iter()
            .map(|field| {
                let location = MetadataLocation::new(field.key.clone());
                Chunk::new(location, field.clone())
            })
            .collect())
    }
}

#[::async_trait::async_trait]
impl DataReader<Metadata> for DocPropsHandler {
    async fn read_at(&self, location: &MetadataLocation) -> Result<Option<MetadataData>> {
        let key = location.key.as_str();
        Ok(self.fields.iter().find(|field| field.key == key).cloned())
    }
}

#[::async_trait::async_trait]
impl DataWriter<Metadata> for DocPropsHandler {
    async fn write_at(&mut self, redactions: Redactions<Metadata>) -> Result<()> {
        for (location, _replacement) in redactions.into_iter() {
            // Every treatment clears the property field: `props::strip` can only
            // blank a field's text, not rewrite it, so a `Replace` degrades to a
            // clear rather than shipping the original value. Clearing
            // unconditionally is fail-closed against any future
            // `MetadataReplacement` variant, no treatment leaves the field intact.
            self.removed.push(location.key.as_str().to_owned());
        }
        Ok(())
    }
}

/// Loader that decodes a property part's bytes into a [`DocPropsHandler`].
#[derive(Debug)]
pub(crate) struct DocPropsLoader;

#[::async_trait::async_trait]
impl Loader for DocPropsLoader {
    type Modality = Metadata;
    type Stream = DocPropsHandler;

    async fn decode(&self, content: ContentData) -> Result<DocPropsHandler> {
        Ok(DocPropsHandler::new(content.to_bytes()))
    }
}

/// [`Format`](elide_codec::Format) descriptor for an OOXML property sub-part.
pub fn format() -> elide_codec::Format {
    elide_codec::Format::new(FORMAT_ID.clone(), DocPropsLoader)
        .with_extensions([PROPS_HINT])
        .with_content_types(["application/x-elide-docprops"])
}

#[cfg(test)]
mod tests {
    use elide_core::entity::{LabelRef, builtins};

    use super::*;

    #[test]
    fn each_field_carries_what_its_schema_says_it_is() {
        let xml = Bytes::from_static(
            br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jane Roe</dc:creator><dc:title>Quarterly plan</dc:title><cp:keywords>acquisition</cp:keywords></cp:coreProperties>"#,
        );
        let handler = DocPropsHandler::new(xml);
        let label = |key: &str| {
            handler
                .fields
                .iter()
                .find(|field| field.key() == key)
                .map(|field| field.label().cloned())
        };
        assert_eq!(
            label("creator"),
            Some(Some(LabelRef::from(&*builtins::PERSON_NAME)))
        );
        assert_eq!(
            label("title"),
            Some(Some(LabelRef::from(&*builtins::COMMUNICATIONS_CONTENT)))
        );
        // A field the schema does not label is not surfaced at all.
        assert_eq!(label("keywords"), None);
    }
}
