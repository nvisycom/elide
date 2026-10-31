//! The PDF [`Recombine`]: applies the recorded redactions from the shared
//! [`PdfState`], folds in the redacted and removed image, scanned-page, and
//! attachment blobs, and checks the output carries no hidden content.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, LocalId, Recombine};
use elide_core::Result;

#[cfg(feature = "render")]
use super::pdf_loader::parse_page_part_id;
use super::pdf_loader::{parse_attachment_part_id, parse_image_part_id};
use super::pdf_state::{Folds, PdfState};
use crate::attachment::AttachmentReplacement;
use crate::document::Pdf;
use crate::redact::ImageReplacement;
#[cfg(feature = "render")]
use crate::redact::PageReplacement;

/// Re-serialises the PDF: applies the recorded redactions (from the shared
/// [`PdfState`]) per its [`RedactMode`](super::RedactMode), folding in the
/// embedded-image, scanned-page, and attachment
/// [`Blob`](elide_codec::DocumentPart::Blob)s alongside.
pub(super) struct PdfRecombine {
    /// The (sanitized) document bytes, retained so [`Pdf`] re-serialises from
    /// the decoded source.
    pub(super) document: Bytes,
    /// The shared redaction state, read to apply the detections.
    pub(super) state: PdfState,
}

impl Recombine for PdfRecombine {
    fn assemble(&self, parts: &[EncodedPart], removed: &[LocalId]) -> Result<ContentData> {
        let mut folds = Folds::default();
        // Each blob part is an embedded image (`img-N-G`), a scanned page
        // (`page-N`), or an attachment (`att-N-G`); the body stream part
        // carries no bytes and matches none.
        for part in parts {
            let id = part.id.as_str();
            if let Some(id) = parse_attachment_part_id(id) {
                folds.attachments.push(AttachmentReplacement {
                    id,
                    bytes: part.bytes.to_vec(),
                });
            }
            if let Some(id) = parse_image_part_id(id) {
                folds.images.push(ImageReplacement {
                    id,
                    image: part.bytes.to_vec(),
                });
            }
            #[cfg(feature = "render")]
            if let Some(number) = parse_page_part_id(id) {
                folds.pages.push(PageReplacement {
                    number,
                    image: part.bytes.to_vec(),
                });
            }
        }
        // A removed blob is content no codec could read: an attachment is
        // dropped, an image removed, and a scanned page cleared, so its pixels
        // never ship unredacted.
        for id in removed {
            let id = id.as_str();
            if let Some(id) = parse_attachment_part_id(id) {
                folds.removed_attachments.push(id);
            }
            if let Some(id) = parse_image_part_id(id) {
                folds.removed_images.push(id);
            }
            #[cfg(feature = "render")]
            if let Some(number) = parse_page_part_id(id) {
                folds.removed_pages.push(number);
            }
        }

        let bytes = self.state.redact(&self.document, folds)?;
        Ok(ContentData::new(bytes))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        Pdf::open(output.as_bytes())?.check_sanitized()
    }
}
