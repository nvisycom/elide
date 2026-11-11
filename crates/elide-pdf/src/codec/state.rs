//! The redaction state: [`PdfInner`] (pages + recorded detections + mode)
//! behind [`PdfState`], which the body stream owns.

use std::ops::Range;

use bytes::Bytes;
use elide_core::Result;

use super::{PdfPage, RedactMode};
use crate::document::{Pdf, Redaction};
use crate::text::{Detection, PageGeometry};

/// The document's editable redaction state, behind [`PdfState`].
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
    fn record(&mut self, number: u32, range: &Range<usize>) {
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

/// The document's editable redaction state, owned by the body
/// [`PdfStream`](super::stream::PdfStream): it records each redaction, and the
/// [`PdfDocument`](super::document::PdfDocument) reads it back to
/// re-serialise.
pub(crate) struct PdfState(PdfInner);

impl PdfState {
    /// Wrap the decoded redaction state.
    pub(super) fn new(inner: PdfInner) -> Self {
        Self(inner)
    }

    /// The extracted pages, in page order.
    pub(super) fn pages(&self) -> &[PdfPage] {
        &self.0.pages
    }

    /// The rendered glyph geometry of the page numbered `number`, for resolving a
    /// detected span to page-normalized highlight rectangles.
    pub(super) fn geometry_for(&self, number: u32) -> Option<&PageGeometry> {
        self.page_by_number(number)?.geometry.as_deref()
    }

    /// The page numbered `number`.
    pub(super) fn page_by_number(&self, number: u32) -> Option<&PdfPage> {
        self.0.pages.iter().find(|p| p.number == number)
    }

    /// Record a redaction at page-local `range` on page `number` (see
    /// [`PdfInner::record`]).
    pub(super) fn record(&mut self, number: u32, range: &Range<usize>) {
        self.0.record(number, range);
    }

    /// Apply the recorded redactions to `document` per the [`RedactMode`]; on
    /// the glyph path, with the redacted and removed blobs in `redaction`,
    /// all in one pass. Returns the original bytes when nothing changes.
    pub(super) fn redact(&self, document: &Bytes, mut redaction: Redaction) -> Result<Bytes> {
        let inner = &self.0;
        match &inner.mode {
            RedactMode::GlyphDelete => {
                redaction.text.clone_from(&inner.deletions);
                if redaction.is_empty() {
                    return Ok(document.clone());
                }
                let out = Pdf::open(document)?.apply(&redaction)?;
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
