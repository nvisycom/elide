//! The shared redaction state: [`PdfInner`] (pages + recorded detections + mode)
//! behind the `Clone`-to-share [`PdfState`] lock.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use elide_core::Result;

use super::{PdfPage, RedactMode};
use crate::attachment::{AttachmentId, AttachmentReplacement};
use crate::document::Pdf;
use crate::redact::Detection;
#[cfg(feature = "image")]
use crate::redact::ImageReplacement;
#[cfg(feature = "render")]
use crate::redact::PageReplacement;
use crate::text::PageGeometry;

/// The document's editable redaction state, behind the shared [`PdfState`] lock.
#[derive(Debug, Default)]
pub(super) struct PdfInner {
    /// Extracted pages, in page order, with stream offsets for `chunks`.
    pub(super) pages: Vec<PdfPage>,
    /// Recorded glyph-deletion detections (per-page character spans), applied at
    /// encode in [`RedactMode::GlyphDelete`].
    pub(super) deletions: Vec<Detection>,
    /// How recorded redactions are applied on encode.
    pub(super) mode: RedactMode,
}

impl PdfInner {
    /// Record a redaction at page-local byte `range` on the page numbered
    /// `number`, as a per-page character-span [`Detection`] routed to the
    /// glyph-delete or raster accumulator per `mode`. A range on no such page, or
    /// off a char boundary, is skipped.
    fn record(&mut self, number: u32, range: &std::ops::Range<usize>) {
        let Some(page) = self.pages.iter().find(|p| p.number == number) else {
            return;
        };
        if page.text.get(range.clone()).is_none() {
            return; // range out of bounds or not on a char boundary
        }
        // Both paths address glyphs by a character span into the page text, so
        // the byte range is measured as character offsets.
        let start = page.text[..range.start].chars().count();
        let end = page.text[..range.end].chars().count();
        let detection = Detection::new(page.number, start, end);
        match &mut self.mode {
            #[cfg(feature = "render")]
            RedactMode::Raster { detections, .. } => detections.push(detection),
            _ => self.deletions.push(detection),
        }
    }
}

/// The document's editable redaction state, shared between the body
/// [`PdfStream`](super::pdf_stream::PdfStream) and the
/// [`PdfRecombine`](super::pdf_recombine::PdfRecombine) so a redaction recorded on
/// the stream is visible when the recombiner re-serialises. `Clone` shares the
/// one state (an `Arc` bump); the lock is held only inside these methods.
#[derive(Clone)]
pub(crate) struct PdfState(Arc<Mutex<PdfInner>>);

impl PdfState {
    /// Wrap the decoded redaction state.
    pub(super) fn new(inner: PdfInner) -> Self {
        Self(Arc::new(Mutex::new(inner)))
    }

    /// The page at `index` in page order, cloned.
    pub(super) fn page(&self, index: usize) -> Option<PdfPage> {
        self.0.lock().unwrap().pages.get(index).cloned()
    }

    /// The number of extracted pages.
    pub(super) fn page_count(&self) -> usize {
        self.0.lock().unwrap().pages.len()
    }

    /// The rendered glyph geometry of the page numbered `number`, for resolving a
    /// detected span to page-normalized highlight rectangles. Returns a cheap
    /// [`Arc`] clone (a refcount bump), not a deep copy of the page's glyph list,
    /// so a lookup per detected entity stays O(1) and the lock is held only for
    /// the lookup.
    pub(super) fn geometry_for(&self, number: u32) -> Option<Arc<PageGeometry>> {
        self.0
            .lock()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.number == number)
            .and_then(|p| p.geometry.clone())
    }

    /// The page numbered `number`, cloned, for reading its text back.
    pub(super) fn page_by_number(&self, number: u32) -> Option<PdfPage> {
        self.0
            .lock()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.number == number)
            .cloned()
    }

    /// Record a redaction at page-local `range` on page `number` (see
    /// [`PdfInner::record`]).
    pub(super) fn record(&self, number: u32, range: &std::ops::Range<usize>) {
        self.0.lock().unwrap().record(number, range);
    }

    /// Apply the recorded redactions to `document` per the [`RedactMode`], then
    /// fold in the redacted and removed blob parts. Returns the original bytes
    /// when nothing was recorded or folded.
    pub(super) fn redact(&self, document: &Bytes, folds: Folds) -> Result<Bytes> {
        let inner = self.0.lock().unwrap();
        match &inner.mode {
            RedactMode::GlyphDelete => {
                if inner.deletions.is_empty() && folds.is_empty() {
                    return Ok(document.clone());
                }

                // Delete the detected glyphs, keeping a selectable text layer.
                let mut out =
                    Pdf::open(document).and_then(|pdf| pdf.redact_text(&inner.deletions))?;

                // Then fold in any redacted embedded images.
                #[cfg(feature = "image")]
                if !folds.images.is_empty() {
                    out = Pdf::open(&out).and_then(|pdf| pdf.redact_images(&folds.images))?;
                }

                // Then reflatten any scanned pages to their redacted raster.
                #[cfg(feature = "render")]
                if !folds.pages.is_empty() {
                    out = Pdf::open(&out).and_then(|pdf| pdf.redact_pages(&folds.pages))?;
                }

                // Then write back the redacted attachments and drop the removed.
                if !folds.attachments.is_empty() || !folds.removed_attachments.is_empty() {
                    out = Pdf::open(&out).and_then(|pdf| {
                        pdf.redact_attachments(&folds.attachments, &folds.removed_attachments)
                    })?;
                }

                Ok(Bytes::from(out))
            }
            #[cfg(feature = "render")]
            RedactMode::Raster {
                observations,
                detections,
            } => {
                if detections.is_empty() {
                    return Ok(document.clone());
                }
                // Fill the detected glyph boxes and emit a fresh image-only PDF,
                // the strong redaction guarantee. Black fill.
                let (out, _certificate) = Pdf::open(document)
                    .and_then(|pdf| pdf.redact_raster(observations, detections, [0, 0, 0]))?;
                Ok(Bytes::from(out))
            }
        }
    }
}

/// The blob parts folded back into a PDF on encode: the redacted embedded
/// images and scanned pages, and the redacted and removed attachments. An image
/// or page whose blob was removed folds back as a blank replacement.
#[derive(Debug, Default)]
pub(super) struct Folds {
    /// Embedded images to replace with their redacted bytes.
    #[cfg(feature = "image")]
    pub(super) images: Vec<ImageReplacement>,
    /// Scanned pages to reflatten to their redacted raster.
    #[cfg(feature = "render")]
    pub(super) pages: Vec<PageReplacement>,
    /// Attachments to replace with their redacted bytes.
    pub(super) attachments: Vec<AttachmentReplacement>,
    /// Attachments to drop from the document.
    pub(super) removed_attachments: Vec<AttachmentId>,
}

impl Folds {
    /// Whether nothing folds back.
    fn is_empty(&self) -> bool {
        #[cfg(feature = "image")]
        let images = self.images.is_empty();
        #[cfg(not(feature = "image"))]
        let images = true;
        #[cfg(feature = "render")]
        let pages = self.pages.is_empty();
        #[cfg(not(feature = "render"))]
        let pages = true;
        images && pages && self.attachments.is_empty() && self.removed_attachments.is_empty()
    }
}
