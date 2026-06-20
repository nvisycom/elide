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

use std::io::{Cursor, Read, Write};

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::DocxLoader;
use crate::content::ContentData;
use crate::handler::extract::Encoder;
use crate::handler::extract::ExtractHandler;
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

/// Re-packs a DOCX: splice the redacted body items back into the body XML,
/// then rebuild the zip with that one part replaced and every other entry
/// copied through verbatim.
#[derive(Debug)]
pub(crate) struct DocxEncoder {
    /// The original package bytes, retained so non-text parts (media,
    /// metadata, relationships) re-pack unchanged.
    pub(super) archive: Bytes,
    /// The raw `word/document.xml` string the items were extracted from.
    pub(super) body_raw: String,
}

impl Encoder for DocxEncoder {
    type Address = XmlSpan;

    fn encode(&self, items: &[XmlItem]) -> Result<ContentData> {
        // 1. Splice the redacted item values back into the body XML.
        let body = xml_splice(&self.body_raw, items)?;

        // 2. Rebuild the zip: copy every entry verbatim except the body
        //    part, which is replaced with the redacted XML.
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
}
