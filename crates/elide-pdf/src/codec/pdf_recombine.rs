//! The PDF [`Recombine`]: applies the recorded redactions from the shared
//! [`PdfState`] and folds in any redacted image / scanned-page blobs.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, Recombine};
use elide_core::Result;

#[cfg(feature = "image")]
use super::BODY_PART_ID;
#[cfg(feature = "image")]
use super::pdf_loader::parse_image_part_id;
#[cfg(feature = "render")]
use super::pdf_loader::parse_page_part_id;
use super::pdf_state::PdfState;
#[cfg(feature = "image")]
use crate::redact::ImageReplacement;
#[cfg(feature = "render")]
use crate::redact::PageReplacement;

/// Re-serialises the PDF: applies the recorded redactions (from the shared
/// [`PdfState`]) per its [`RedactMode`](super::RedactMode), folding in any
/// redacted embedded-image or scanned-page [`Blob`](elide_codec::DocumentPart::Blob)s
/// alongside.
pub(super) struct PdfRecombine {
    /// The original document bytes, retained so [`Pdf`](crate::document::Pdf) re-serialises
    /// from the true source.
    pub(super) document: Bytes,
    /// The shared redaction state, read to apply the detections.
    pub(super) state: PdfState,
}

impl Recombine for PdfRecombine {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        // Without an image codec no blob parts are surfaced, so the part list is
        // unused: only glyph deletions and the raster path apply.
        #[cfg(not(feature = "image"))]
        let _ = parts;

        // The redacted embedded images (`img-N-G`) and scanned pages (`page-N`)
        // come back as blob parts; the body stream part carries no bytes.
        #[cfg(feature = "image")]
        let image_replacements: Vec<ImageReplacement> = parts
            .iter()
            .filter(|p| p.id.as_str() != BODY_PART_ID)
            .filter_map(|p| {
                Some(ImageReplacement {
                    id: parse_image_part_id(p.id.as_str())?,
                    image: p.bytes.to_vec(),
                })
            })
            .collect();
        #[cfg(feature = "render")]
        let page_replacements: Vec<PageReplacement> = parts
            .iter()
            .filter(|p| p.id.as_str() != BODY_PART_ID)
            .filter_map(|p| {
                Some(PageReplacement {
                    number: parse_page_part_id(p.id.as_str())?,
                    image: p.bytes.to_vec(),
                })
            })
            .collect();

        let bytes = self.state.redact(
            &self.document,
            #[cfg(feature = "image")]
            image_replacements,
            #[cfg(feature = "render")]
            page_replacements,
        )?;
        Ok(ContentData::new(bytes))
    }
}
