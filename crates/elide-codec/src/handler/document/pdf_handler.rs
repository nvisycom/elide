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
#[cfg(feature = "pdf-render")]
use elide_core::modality::image::ImageData;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

#[cfg(feature = "pdf-render")]
use super::OcrMode;
use super::PdfLoader;
use crate::codec::{Container, Part, PartId};
use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the PDF codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// Decodes on the plain text path (no page rendering). To rasterise pages
/// for OCR, build the format with [`format_with_ocr`] instead.
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PdfLoader::new())
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// [`Format`] descriptor that rasterises PDF pages for OCR per `ocr`.
///
/// Mirrors the force-OCR switch other tools expose (OCRmyPDF `--force-ocr`,
/// Docling `force_full_page_ocr`): under [`OcrMode::Force`] every page is
/// rendered to an image on decode for the image/OCR pipeline.
///
/// [`OcrMode::Force`]: super::OcrMode::Force
#[cfg(feature = "pdf-render")]
#[cfg_attr(docsrs, doc(cfg(feature = "pdf-render")))]
pub fn format_with_ocr(ocr: OcrMode) -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PdfLoader::with_ocr(ocr))
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// Stub text handler that may also carry pages rendered for OCR.
///
/// Text parsing is still a stub (no chunks, no parts). When the loader runs
/// under [`OcrMode::Force`], it rasterises the pages up front and hands them
/// here as [`ImageData`]; those ride along for the image/OCR pipeline to
/// consume (the part/media hookup lands with XObject extraction).
///
/// [`OcrMode::Force`]: super::OcrMode::Force
#[derive(Debug, Default)]
pub(crate) struct PdfHandler {
    /// Pages rasterised for OCR, present only under the `pdf-render` feature
    /// and [`OcrMode::Force`]; empty on the plain text path.
    ///
    /// [`OcrMode::Force`]: super::OcrMode::Force
    #[cfg(feature = "pdf-render")]
    pages: Vec<ImageData>,
}

impl PdfHandler {
    /// A handler with no rendered pages (the plain text path).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A handler carrying pages rasterised for OCR.
    #[cfg(feature = "pdf-render")]
    pub(crate) fn with_pages(pages: Vec<ImageData>) -> Self {
        Self { pages }
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
        // Pages rendered for OCR (under the `pdf-render` feature and
        // `OcrMode::Force`) surface as image parts: the orchestrator decodes
        // each PNG back to an `Image` and drives the OCR pipeline over it,
        // exactly like DOCX media parts. Without rendering this is empty —
        // XObject extraction of native PDF images is still to come.
        #[cfg(feature = "pdf-render")]
        {
            self.pages
                .iter()
                .enumerate()
                .map(|(index, page)| Part {
                    id: PartId::from(format!("page-{index}")),
                    bytes: page.bytes.clone(),
                    hint: "png".to_string(),
                })
                .collect()
        }
        #[cfg(not(feature = "pdf-render"))]
        Vec::new()
    }

    fn replace_part(&mut self, id: &PartId, _bytes: Bytes) -> Result<()> {
        // Rendered pages are detection-only inputs: folding redactions back
        // requires re-encoding the PDF, which is not supported yet (see
        // `encode`). So no part is writable today.
        Err(Error::new(
            ErrorKind::Validation,
            format!("pdf replace_part: `{id}` is not a writable part"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Loader;

    #[tokio::test]
    async fn stub_decodes_but_exposes_nothing() {
        let mut h = PdfLoader::new()
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

    // The OCR-forcing format builds, and the `Auto` path decodes without
    // rendering (so no native library is needed here).
    #[cfg(feature = "pdf-render")]
    #[tokio::test]
    async fn auto_mode_decodes_without_rendering() {
        use super::super::OcrMode;

        let _ = format_with_ocr(OcrMode::force());

        let h = PdfLoader::with_ocr(OcrMode::Auto)
            .decode(ContentData::new(Bytes::from_static(b"%PDF-1.7")))
            .await
            .unwrap();
        assert!(h.parts().is_empty());
    }

    // The `Force` decode path renders via the native PDFium library, absent
    // in CI — ignored by default; run with `--ignored` where PDFium is
    // installed (see `scripts/install-pdfium.sh`). Renders a minimal
    // one-page PDF and checks a page image came back.
    #[cfg(feature = "pdf-render")]
    #[tokio::test]
    #[ignore = "requires the native PDFium shared library at runtime"]
    async fn force_mode_renders_pages() {
        use super::super::OcrMode;

        const MINIMAL_PDF: &[u8] = b"%PDF-1.4\n\
            1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
            2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n\
            3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\n\
            trailer<</Root 1 0 R>>\n%%EOF";

        let h = PdfLoader::with_ocr(OcrMode::force())
            .decode(ContentData::new(Bytes::from_static(MINIMAL_PDF)))
            .await
            .unwrap();
        // The rendered page surfaces as a decodable image part.
        let parts = h.parts();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].hint, "png");
        assert!(!parts[0].bytes.is_empty());
    }
}
