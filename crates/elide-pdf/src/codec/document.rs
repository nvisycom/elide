//! [`PdfDocument`]: a decoded PDF, re-serialised by applying the redactions its
//! body stream recorded,
//! folds in the redacted and removed image, scanned-page, and
//! attachment blobs, and checks the output carries no hidden content.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{Blob, LocalId, Native, StreamMut};
use elide_core::Result;
use elide_core::modality::text::Text;

use super::BODY_PART_ID;
use super::part_id::PartId;
use super::stream::PdfStream;
use crate::document::attachment::AttachmentReplacement;
use crate::document::image::ImageReplacement;
use crate::document::{Pdf, Redaction};
#[cfg(feature = "render")]
use crate::render::PageReplacement;

/// A decoded PDF: its page-text body stream over the retained document,
/// re-serialised by applying the redactions the stream recorded, per its
/// [`RedactMode`](super::RedactMode), folding in the embedded-image,
/// scanned-page, and attachment [`Blob`]s alongside.
pub(super) struct PdfDocument {
    /// The page-text body.
    pub(super) body: PdfStream,
    /// The (sanitized) document bytes, retained so [`Pdf`] re-serialises from
    /// the decoded source.
    pub(super) document: Bytes,
}

impl Native for PdfDocument {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        vec![(
            LocalId::new(BODY_PART_ID),
            StreamMut::new::<Text, _>(&mut self.body),
        )]
    }

    fn encode(&self, blobs: &[Blob], removed: &[LocalId]) -> Result<ContentData> {
        let mut redaction = Redaction::default();
        // Each blob is an embedded image, a scanned page, or an attachment.
        for part in blobs {
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

        let bytes = self.body.state.redact(&self.document, redaction)?;
        Ok(ContentData::new(bytes))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        Pdf::open(output.as_bytes())?.check_sanitized()
    }
}
