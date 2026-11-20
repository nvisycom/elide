//! [`PdfDocument`]: a decoded PDF, re-serialised by applying the redactions its
//! body and scanned-page streams recorded, with the redacted and removed image
//! and attachment blobs folded in, and checked to carry no hidden content.

use std::collections::BTreeMap;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{Blob, LocalId, Native, StreamMut};
use elide_core::Result;
use elide_core::modality::text::Text;
#[cfg(feature = "render")]
use elide_image::modality::Image;

use super::BODY_PART_ID;
use super::blob_id::BlobId;
#[cfg(feature = "render")]
use super::page::PageStream;
use super::plan::Plan;
use super::stream::PdfStream;
use crate::content::attachment::AttachmentReplacement;
use crate::content::image::ImageReplacement;
use crate::pdf::Pdf;
use crate::sanitize::Check;

/// A decoded PDF: its page-text body stream over the rebuilt document, and
/// (feature `render`) an image stream per scanned page, with the
/// embedded-image and attachment [`Blob`]s alongside.
pub(super) struct PdfDocument {
    /// The page-text body.
    pub(super) body: PdfStream,
    /// Each scanned page's render, in page order.
    #[cfg(feature = "render")]
    pub(super) pages: Vec<PageStream>,
    /// The rebuilt document, parsed and saved: encode edits a copy of it, and
    /// ships its bytes when nothing is redacted.
    pub(super) pdf: Pdf,
    /// Each blob's bytes as surfaced, by id: a blob that comes back with the
    /// same bytes is left as the document holds it.
    pub(super) surfaced: BTreeMap<String, Bytes>,
}

impl Native for PdfDocument {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        #[cfg_attr(not(feature = "render"), expect(unused_mut))]
        let mut streams = vec![(
            LocalId::new(BODY_PART_ID),
            StreamMut::new::<Text, _>(&mut self.body),
        )];
        #[cfg(feature = "render")]
        streams.extend(
            self.pages
                .iter_mut()
                .map(|page| (page.id(), StreamMut::new::<Image, _>(page))),
        );
        streams
    }

    fn encode(&self, blobs: &[Blob], removed: &[LocalId]) -> Result<ContentData> {
        let mut plan = Plan::default();
        #[cfg(feature = "render")]
        {
            plan.scan_scale = self.body.state.scan_scale();
            for page in &self.pages {
                plan.scanned.extend(page.scanned()?);
            }
        }
        for part in blobs {
            let surfaced = self.surfaced.get(part.id.as_str());
            if surfaced == Some(&part.bytes) {
                continue;
            }
            let bytes = part.bytes.to_vec();
            match BlobId::parse(part.id.as_str())? {
                BlobId::Image(id) => {
                    plan.images.push(ImageReplacement { id, image: bytes });
                }
                BlobId::Attachment(id) => {
                    plan.attachments.push(AttachmentReplacement { id, bytes });
                }
            }
        }
        // A removed blob is content no codec could read: an attachment is
        // dropped and an image removed, so it never ships unredacted.
        for id in removed {
            match BlobId::parse(id.as_str())? {
                BlobId::Image(id) => plan.removed_images.push(id),
                BlobId::Attachment(id) => plan.removed_attachments.push(id),
            }
        }

        let bytes = self.body.state.redact(&self.pdf, plan)?;
        Ok(ContentData::new(bytes))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        Pdf::open(output.as_bytes())?.read(Check)
    }
}
