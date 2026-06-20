//! DOCX handler side: the [`DocxHandler`] type, its [`Format`]
//! descriptor, and the [`DocxEncoder`] that re-packs the zip after the
//! body XML has been redacted.
//!
//! The handler *is* an [`ExtractHandler`] over the items extracted from
//! `word/document.xml`. Redaction edits those item values in place; on
//! encode, [`DocxEncoder`] splices them back into the body XML and
//! rebuilds the zip, copying every other entry through unchanged so the
//! container round-trips byte-for-byte except for the redacted text.
//!
//! [`ExtractHandler`]: crate::handler::extract::ExtractHandler

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::DocxLoader;
use crate::content::ContentData;
use crate::handler::extract::{Encoder, ExtractHandler};
use crate::handler::markup::{XmlItem, XmlSpan, xml_splice};
use crate::{Format, FormatId};

/// Stable [`FormatId`] for the DOCX codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.docx");

/// The OOXML part holding the main document body text.
pub(super) const BODY_PART: &str = "word/document.xml";

/// Handler type for loaded DOCX content.
pub(crate) type DocxHandler = ExtractHandler<DocxEncoder>;

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<elide_core::modality::text::Text, _>(FORMAT_ID.clone(), DocxLoader)
        .with_extensions(["docx"])
        .with_content_types([
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ])
}

/// The OOXML media directory whose entries are embedded binary parts
/// (images). The `Container` impl exposes these for out-of-band redaction.
pub(super) const MEDIA_PREFIX: &str = "word/media/";

/// Re-packs a DOCX: splice the redacted body items back into the body XML,
/// fold in any replaced media parts, and copy every other entry through
/// verbatim.
#[derive(Debug)]
pub(crate) struct DocxEncoder {
    /// The original package bytes, retained so non-text parts (media,
    /// metadata, relationships) re-pack unchanged.
    pub(super) archive: Bytes,
    /// The raw `word/document.xml` string the items were extracted from.
    pub(super) body_raw: String,
    /// Redacted replacements for media parts, keyed by zip entry name,
    /// filled through the [`Container`](crate::codec::Container) surface.
    pub(super) replacements: HashMap<String, Bytes>,
}

impl Encoder for DocxEncoder {
    type Address = XmlSpan;

    fn encode(&self, items: &[XmlItem]) -> Result<ContentData> {
        // 1. Splice the redacted item values back into the body XML.
        let body = xml_splice(&self.body_raw, items)?;

        // 2. Rebuild the zip: the body part becomes the redacted XML, a
        //    replaced media part becomes its redacted bytes, and every
        //    other entry is copied through verbatim.
        let mut reader = ZipArchive::new(Cursor::new(self.archive.as_ref()))
            .map_err(|e| Error::new(ErrorKind::Validation, format!("malformed docx zip: {e}")))?;
        let mut out = ZipWriter::new(Cursor::new(Vec::new()));

        for i in 0..reader.len() {
            let mut entry = reader
                .by_index(i)
                .map_err(|e| Error::new(ErrorKind::Validation, format!("docx entry {i}: {e}")))?;
            let name = entry.name().to_owned();
            let options = SimpleFileOptions::default().compression_method(entry.compression());
            out.start_file(&name, options).map_err(|e| {
                Error::new(ErrorKind::Validation, format!("docx repack {name}: {e}"))
            })?;
            if name == BODY_PART {
                out.write_all(body.as_bytes())
                    .map_err(|e| Error::new(ErrorKind::Validation, format!("docx body: {e}")))?;
            } else if let Some(redacted) = self.replacements.get(&name) {
                out.write_all(redacted).map_err(|e| {
                    Error::new(ErrorKind::Validation, format!("docx media {name}: {e}"))
                })?;
            } else {
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf).map_err(|e| {
                    Error::new(ErrorKind::Validation, format!("docx read {name}: {e}"))
                })?;
                out.write_all(&buf).map_err(|e| {
                    Error::new(ErrorKind::Validation, format!("docx copy {name}: {e}"))
                })?;
            }
        }

        let cursor = out
            .finish()
            .map_err(|e| Error::new(ErrorKind::Validation, format!("docx finalize: {e}")))?;
        Ok(ContentData::new(Bytes::from(cursor.into_inner())))
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn crate::codec::Container> {
        Some(self)
    }
}

impl crate::codec::Container for DocxEncoder {
    fn parts(&self) -> Vec<crate::codec::Part> {
        let Ok(mut zip) = ZipArchive::new(Cursor::new(self.archive.as_ref())) else {
            return Vec::new();
        };
        let mut parts = Vec::new();
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let name = entry.name().to_owned();
            if !name.starts_with(MEDIA_PREFIX) {
                continue;
            }
            let mut buf = Vec::with_capacity(entry.size() as usize);
            if entry.read_to_end(&mut buf).is_err() {
                continue;
            }
            let hint = name
                .rsplit_once('.')
                .map(|(_, e)| e.to_owned())
                .unwrap_or_default();
            parts.push(crate::codec::Part {
                id: name,
                bytes: Bytes::from(buf),
                hint,
            });
        }
        parts
    }

    fn replace_part(&mut self, id: &str, bytes: Bytes) -> Result<()> {
        if !id.starts_with(MEDIA_PREFIX) {
            return Err(Error::new(
                ErrorKind::Validation,
                format!("docx replace_part: `{id}` is not a media part"),
            ));
        }
        self.replacements.insert(id.to_owned(), bytes);
        Ok(())
    }
}
