//! PDF handler: a stub today.
//!
//! Decoding succeeds (so the format resolves and round-trips through the
//! registry), but no PDF content is parsed yet: streaming yields nothing,
//! reads return nothing, redaction is a no-op, `encode` reports that
//! re-serialization is unsupported, and the [`Container`] surface exposes
//! no parts. A real implementation (a PDF object parser to read page text
//! and image XObjects, a writer to re-emit) will replace this when PDF
//! extraction lands.
//!
//! Unlike DOCX, a PDF is *not* a zip: it is a flat file of indirect
//! objects with a cross-reference table, and embedded images live as
//! stream objects (image XObjects), not package entries. So PDF brings its
//! own object parser rather than reusing the zip-based container plumbing;
//! only the modality-neutral [`Container`]/[`Part`]/[`PartId`] surface is
//! shared with DOCX.
//!
//! [`Part`]: crate::codec::Part
//! [`PartId`]: crate::codec::PartId

use bytes::Bytes;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

use super::PdfLoader;
use crate::codec::{Container, Part, PartId};
use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the PDF codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PdfLoader)
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// Stub handler: holds nothing and exposes no text or parts. See the
/// module docs.
#[derive(Debug, Default)]
pub(crate) struct PdfHandler;

impl PdfHandler {
    /// An empty stub handler.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Handler<Text> for PdfHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Err(Error::new(
            ErrorKind::Validation,
            "PDF re-encoding is not yet supported",
        ))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        Ok(None)
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Some(self)
    }

    // No `lift` override: the stub yields no chunks, so it is never called;
    // the identity default suffices.
}

impl DataReader<Text> for PdfHandler {
    async fn read_at(&self, _location: &TextLocation) -> Result<Option<TextData>> {
        Ok(None)
    }
}

impl DataWriter<Text> for PdfHandler {
    async fn write_at(&mut self, _redactions: Redactions<Text>) -> Result<()> {
        Ok(())
    }
}

impl Container for PdfHandler {
    fn parts(&self) -> Vec<Part> {
        // No XObject extraction yet — the stub exposes no embedded parts.
        Vec::new()
    }

    fn replace_part(&mut self, id: &PartId, _bytes: Bytes) -> Result<()> {
        // The stub surfaces no parts, so any id is unknown.
        Err(Error::new(
            ErrorKind::Validation,
            format!("pdf replace_part: `{id}` is not a known part"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Loader;

    #[tokio::test]
    async fn stub_decodes_but_exposes_nothing() {
        let mut h = PdfLoader
            .decode(ContentData::new(Bytes::from_static(b"%PDF-1.7")))
            .await
            .unwrap();
        assert_eq!(h.format().as_str(), "elide.document.pdf");
        // No text, no reads, redaction is a no-op, encode is unsupported.
        assert!(h.read_next().await.unwrap().is_none());
        assert!(h.read_at(&TextLocation::new(0, 0)).await.unwrap().is_none());
        assert!(h.encode().is_err());
        // It is a container, but exposes no parts and rejects replacements.
        assert!(h.parts().is_empty());
        assert!(
            h.replace_part(&PartId::new("anything"), Bytes::new())
                .is_err()
        );
    }
}
