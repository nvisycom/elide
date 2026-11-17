//! [`PdfDocument`]: a decoded PDF, re-serialised by applying the redactions its
//! body stream recorded, with the redacted and removed image, scanned-page, and
//! attachment blobs folded in, and checked to carry no hidden content.

use std::collections::BTreeMap;

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
use crate::render::ScannedPage;

/// A decoded PDF: its page-text body stream over the rebuilt document, with
/// the embedded-image, scanned-page, and attachment [`Blob`]s alongside.
pub(super) struct PdfDocument {
    /// The page-text body.
    pub(super) body: PdfStream,
    /// The rebuilt document, parsed and saved: encode edits a copy of it, and
    /// ships its bytes when nothing is redacted.
    pub(super) pdf: Pdf,
    /// Each blob's bytes as surfaced, by id: a blob that comes back with the
    /// same bytes is left as the document holds it.
    pub(super) surfaced: BTreeMap<String, Bytes>,
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
        #[cfg(feature = "render")]
        {
            redaction.scan_scale = self.body.state.scan_scale();
        }
        for part in blobs {
            let surfaced = self.surfaced.get(part.id.as_str());
            if surfaced == Some(&part.bytes) {
                continue;
            }
            let bytes = part.bytes.to_vec();
            match PartId::parse(part.id.as_str())? {
                PartId::Image(id) => {
                    redaction.images.push(ImageReplacement { id, image: bytes });
                }
                PartId::Attachment(id) => {
                    redaction
                        .attachments
                        .push(AttachmentReplacement { id, bytes });
                }
                #[cfg(feature = "render")]
                PartId::Page(number) => redaction.scanned.push(ScannedPage {
                    number,
                    original: surfaced.cloned().unwrap_or_default(),
                    redacted: part.bytes.clone(),
                }),
            }
        }
        // A removed blob is content no codec could read: an attachment is
        // dropped, an image removed, and a scanned page cleared, so its pixels
        // never ship unredacted.
        for id in removed {
            match PartId::parse(id.as_str())? {
                PartId::Image(id) => redaction.removed_images.push(id),
                PartId::Attachment(id) => redaction.removed_attachments.push(id),
                #[cfg(feature = "render")]
                PartId::Page(number) => redaction.cleared_pages.push(number),
            }
        }

        let bytes = self.body.state.redact(&self.pdf, redaction)?;
        Ok(ContentData::new(bytes))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        Pdf::open(output.as_bytes())?.check_rebuilt()
    }
}
