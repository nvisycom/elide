//! The PDF [`Recombine`]: applies the recorded redactions from the shared
//! [`PdfState`], folds in the redacted and removed image, scanned-page, and
//! attachment blobs, and checks the output carries no hidden content.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, LocalId, Recombine};
use elide_core::Result;

use super::part_id::PartId;
use super::state::PdfState;
use crate::document::attachment::AttachmentReplacement;
use crate::document::image::ImageReplacement;
use crate::document::{Pdf, Redaction};
#[cfg(feature = "render")]
use crate::render::PageReplacement;

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
        let mut redaction = Redaction::default();
        // Each blob part is an embedded image, a scanned page, or an attachment;
        // the body stream part carries no bytes and parses as none.
        for part in parts {
            let bytes = part.bytes.to_vec();
            match PartId::parse(part.id.as_str()) {
                Some(PartId::Image(id)) => {
                    redaction.images.push(ImageReplacement { id, image: bytes });
                }
                Some(PartId::Attachment(id)) => {
                    redaction
                        .attachments
                        .push(AttachmentReplacement { id, bytes });
                }
                #[cfg(feature = "render")]
                Some(PartId::Page(number)) => {
                    redaction.pages.push(PageReplacement {
                        number,
                        image: bytes,
                    });
                }
                _ => {}
            }
        }
        // A removed blob is content no codec could read: an attachment is
        // dropped, an image removed, and a scanned page cleared, so its pixels
        // never ship unredacted.
        for id in removed {
            match PartId::parse(id.as_str()) {
                Some(PartId::Image(id)) => redaction.removed_images.push(id),
                Some(PartId::Attachment(id)) => redaction.removed_attachments.push(id),
                #[cfg(feature = "render")]
                Some(PartId::Page(number)) => redaction.cleared_pages.push(number),
                _ => {}
            }
        }

        let bytes = self.state.redact(&self.document, redaction)?;
        Ok(ContentData::new(bytes))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        Pdf::open(output.as_bytes())?.check_sanitized()
    }
}
