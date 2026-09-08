//! Handler and loader for an OOXML document-property part, a `docProps/*`
//! sub-part shared by docx, pptx, and xlsx.
//!
//! Every OOXML package exposes its property parts (`docProps/core.xml`,
//! `docProps/app.xml`) as nested sub-parts, the same way it exposes an embedded
//! image. This handler decodes such a part as the [`Metadata`] modality: it
//! streams each privacy-relevant property field (author, last editor, timestamps,
//! company, manager) as a chunk for the recognizer, records the fields picked for
//! removal via [`write_at`], and re-encodes the property XML with exactly those
//! fields cleared. The owning package handler then folds the edited part back in.
//!
//! [`Metadata`]: elide_core::modality::metadata::Metadata
//! [`write_at`]: elide_core::modality::DataWriter::write_at

use bytes::Bytes;
use elide_core::Result;
use elide_core::entity::{LabelRef, builtins};
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, RecognizerId};
use elide_core::redaction::Redactions;
use elide_office::opc::props;

use crate::content::ContentData;
use crate::{FormatId, Handler, Loader};

/// The reader id recorded on each field's detection audit event.
const SOURCE: &str = "docprops";

/// The label an OOXML document-property field (by local name) carries, or `None` when it is
/// not one this codec treats as privacy-relevant. Shared by the recognizer and
/// any direct read.
fn label_for(key: &str) -> Option<LabelRef> {
    let label = match key {
        "creator" | "lastModifiedBy" | "Manager" => &builtins::PERSON_NAME,
        "created" | "modified" => &builtins::DATE_TIME,
        "Company" => &builtins::ORGANIZATION_NAME,
        // title/subject/description/keywords/category are document-content
        // fields, not identity data, so they are left to a content recognizer.
        _ => return None,
    };
    Some(LabelRef::from(&**label))
}

/// Recognizes a privacy-relevant OOXML document-property field.
///
/// A metadata chunk is one property; this classifies it by local name through
/// the shared label table and, for an author/editor/company/timestamp, emits a
/// single [`Entity`](elide_core::entity::Entity). An unrecognized field (a
/// revision count, an app version) yields nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct DocPropsRecognizer;

#[::async_trait::async_trait]
impl Recognizer<Metadata> for DocPropsRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(SOURCE, env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        data: &MetadataData,
        _ctx: &RecognizerContext<'_, Metadata>,
    ) -> Result<Recognition<Metadata>> {
        let entities = label_for(data.key())
            .and_then(|label| Metadata::field_entity(data.key(), label, SOURCE))
            .into_iter()
            .collect();
        Ok(Recognition::new(entities))
    }
}

/// Stable [`FormatId`](crate::FormatId) for an OOXML property sub-part.
pub const FORMAT_ID: crate::FormatId = crate::FormatId::new("elide.office.docprops");

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

/// Handler for an OOXML property part: streams fields, clears chosen ones,
/// re-encodes the property XML.
#[derive(Debug)]
pub(crate) struct DocPropsHandler {
    /// The property part's XML bytes.
    xml: Bytes,
    /// Every field, parsed once at decode and kept in document order so
    /// [`read_at`](DataReader::read_at) can look one up without re-parsing the
    /// XML.
    fields: Vec<MetadataData>,
    /// The fields yet to stream, drained by `read_next` (reversed for pop).
    pending: Vec<MetadataData>,
    /// Field keys (local names) picked for removal, applied on `encode`.
    removed: Vec<String>,
}

impl DocPropsHandler {
    /// Wrap a property part's XML, priming its fields for streaming.
    pub(crate) fn new(xml: Bytes) -> Self {
        let fields: Vec<MetadataData> = props::fields(&xml)
            .into_iter()
            .map(|(key, value)| MetadataData::new(key, value))
            .collect();
        let mut pending = fields.clone();
        pending.reverse(); // popped, so reverse for first-field-first order
        Self {
            xml,
            fields,
            pending,
            removed: Vec::new(),
        }
    }
}

#[::async_trait::async_trait]
impl Handler<Metadata> for DocPropsHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        let keys: Vec<&str> = self.removed.iter().map(String::as_str).collect();
        Ok(ContentData::new(props::strip(&self.xml, &keys)))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Metadata>>> {
        let Some(field) = self.pending.pop() else {
            return Ok(None);
        };
        let location = MetadataLocation::new(field.key.clone());
        Ok(Some(Chunk::new(location, field)))
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
impl Loader<Metadata> for DocPropsLoader {
    type Handler = DocPropsHandler;

    async fn decode(&self, content: ContentData) -> Result<DocPropsHandler> {
        Ok(DocPropsHandler::new(content.to_bytes()))
    }
}

/// [`Format`](crate::Format) descriptor for an OOXML property sub-part.
pub fn format() -> crate::Format {
    crate::Format::new::<Metadata, _>(FORMAT_ID.clone(), DocPropsLoader)
        .with_extensions([PROPS_HINT])
        .with_content_types(["application/x-elide-docprops"])
}
