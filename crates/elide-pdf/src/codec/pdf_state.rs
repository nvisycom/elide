//! The shared redaction state: [`PdfInner`] (pages + recorded detections + mode)
//! behind the `Clone`-to-share [`PdfState`] lock.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use elide_core::Result;

use super::{PdfPage, RedactMode};
use crate::document::Pdf;
use crate::redact::Detection;
#[cfg(feature = "image")]
use crate::redact::ImageReplacement;
#[cfg(feature = "render")]
use crate::redact::PageReplacement;

/// The document's editable redaction state, behind the shared [`PdfState`] lock.
#[derive(Debug, Default)]
pub(super) struct PdfInner {
    /// Extracted pages, in page order, with stream offsets for `read_next`.
    pub(super) pages: Vec<PdfPage>,
    /// Recorded glyph-deletion detections (per-page character spans), applied at
    /// encode in [`RedactMode::GlyphDelete`].
    pub(super) deletions: Vec<Detection>,
    /// How recorded redactions are applied on encode.
    pub(super) mode: RedactMode,
}

impl PdfInner {
    /// Record a redaction at decoded stream `range` as a per-page character-span
    /// [`Detection`], routed to the glyph-delete or raster accumulator per
    /// `mode`. A range that falls on no page, or off a char boundary, is skipped
    /// (PDF text is addressed by decoded range only; a source-only location has
    /// none).
    fn record(&mut self, range: &std::ops::Range<usize>) {
        let Some((page_idx, local, local_end)) =
            self.pages.iter().enumerate().find_map(|(idx, page)| {
                if range.start < page.start || range.start >= page.start + page.text.len() {
                    return None;
                }
                let local = range.start - page.start;
                let local_end = range.end.checked_sub(page.start)?;
                Some((idx, local, local_end))
            })
        else {
            return;
        };
        let page = &self.pages[page_idx];
        if page.text.get(local..local_end).is_none() {
            return; // range not on a char boundary
        }
        // Both paths address glyphs by the same character span into the page
        // text, so the span is measured once as character offsets.
        let start = page.text[..local].chars().count();
        let end = page.text[..local_end].chars().count();
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

    /// The page whose stream range contains `offset`, and the offset within it.
    pub(super) fn page_at(&self, offset: usize) -> Option<(PdfPage, usize)> {
        let inner = self.0.lock().unwrap();
        inner
            .pages
            .iter()
            .find(|p| offset >= p.start && offset < p.start + p.text.len())
            .map(|p| (p.clone(), offset - p.start))
    }

    /// Record a redaction at decoded stream `range` (see [`PdfInner::record`]).
    pub(super) fn record(&self, range: &std::ops::Range<usize>) {
        self.0.lock().unwrap().record(range);
    }

    /// Apply the recorded redactions to `document` per the [`RedactMode`], folding
    /// in the redacted `image_replacements` and (scanned) `page_replacements`.
    /// Returns the original bytes when nothing was recorded.
    pub(super) fn redact(
        &self,
        document: &Bytes,
        #[cfg(feature = "image")] image_replacements: Vec<ImageReplacement>,
        #[cfg(feature = "render")] page_replacements: Vec<PageReplacement>,
    ) -> Result<Bytes> {
        let inner = self.0.lock().unwrap();
        match &inner.mode {
            RedactMode::GlyphDelete => {
                #[cfg(feature = "image")]
                let has_images = !image_replacements.is_empty();
                #[cfg(not(feature = "image"))]
                let has_images = false;
                #[cfg(feature = "render")]
                let has_pages = !page_replacements.is_empty();
                #[cfg(not(feature = "render"))]
                let has_pages = false;
                if inner.deletions.is_empty() && !has_images && !has_pages {
                    return Ok(document.clone());
                }

                // Delete the detected glyphs and strip annotations/metadata,
                // keeping a selectable text layer. (`mut` is used only when the
                // image fold below is compiled in.)
                #[cfg_attr(not(feature = "image"), allow(unused_mut))]
                let mut out =
                    Pdf::open(document).and_then(|pdf| pdf.redact_text(&inner.deletions))?;

                // Then fold in any redacted embedded images.
                #[cfg(feature = "image")]
                if has_images {
                    out = Pdf::open(&out).and_then(|pdf| pdf.redact_images(&image_replacements))?;
                }

                // Then reflatten any scanned pages to their redacted raster.
                #[cfg(feature = "render")]
                if has_pages {
                    out = Pdf::open(&out).and_then(|pdf| pdf.redact_pages(&page_replacements))?;
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
