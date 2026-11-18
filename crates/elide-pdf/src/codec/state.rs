//! [`PdfState`]: a decoded PDF's pages and the redactions recorded on them,
//! which the body stream owns and the document applies on encode.

use std::ops::Range;

use bytes::Bytes;
use elide_core::{Error, Result};

#[cfg(feature = "render")]
use super::SCAN_DPI;
use super::plan::Plan;
use crate::content::text::{Detection, TextBlock};
use crate::pdf::Pdf;
use crate::pdf::reader::PageGeometry;
#[cfg(feature = "render")]
use crate::pdf::render::PageObservation;

/// A decoded PDF's pages, as the redaction mode reads them.
#[derive(Debug)]
pub(super) enum Pages {
    /// Glyph deletion: each page's text with the map back to the glyphs that
    /// drew it.
    Glyph(Vec<TextBlock>),
    /// Flatten (feature `render`): every page's text as PDFium reads it, with
    /// each glyph's box; encode renders the pages at `scale`.
    #[cfg(feature = "render")]
    Flatten {
        pages: Vec<PageObservation>,
        scale: f32,
    },
}

/// A decoded PDF's pages and the redactions recorded on them.
#[derive(Debug)]
pub(super) struct PdfState {
    pages: Pages,
    /// Recorded detections (page + character span), applied at encode.
    detections: Vec<Detection>,
}

impl PdfState {
    /// The state of `pages`, with nothing recorded.
    pub(super) fn new(pages: Pages) -> Self {
        Self {
            pages,
            detections: Vec::new(),
        }
    }

    /// Each page's number and text, in page order, leaving out a page with no
    /// text (a scanned page, surfaced as an image instead).
    pub(super) fn texts(&self) -> Vec<(u32, &str)> {
        let texts: Vec<(u32, &str)> = match &self.pages {
            Pages::Glyph(blocks) => blocks.iter().map(|b| (b.page, b.text.as_str())).collect(),
            #[cfg(feature = "render")]
            Pages::Flatten { pages, .. } => {
                pages.iter().map(|p| (p.page, p.text.as_str())).collect()
            }
        };
        texts
            .into_iter()
            .filter(|(_, text)| !text.trim().is_empty())
            .collect()
    }

    /// The text of the page numbered `number`.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] for a page that has no text.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn text(&self, number: u32) -> Result<&str> {
        self.texts()
            .into_iter()
            .find_map(|(page, text)| (page == number).then_some(text))
            .ok_or_else(|| Error::malformed(format!("no page {number}")))
    }

    /// The glyph boxes and frame of the page numbered `number`.
    pub(super) fn geometry(&self, number: u32) -> Option<PageGeometry<'_>> {
        match &self.pages {
            Pages::Glyph(blocks) => blocks
                .iter()
                .find(|b| b.page == number)
                .map(TextBlock::geometry),
            #[cfg(feature = "render")]
            Pages::Flatten { pages, .. } => pages
                .iter()
                .find(|p| p.page == number)
                .map(PageObservation::geometry),
        }
    }

    /// The scale a scanned page was rendered at.
    #[cfg(feature = "render")]
    pub(super) fn scan_scale(&self) -> f32 {
        match &self.pages {
            Pages::Glyph(_) => SCAN_DPI.scale_factor(),
            Pages::Flatten { scale, .. } => *scale,
        }
    }

    /// The detections recorded so far.
    #[cfg(test)]
    pub(super) fn detections(&self) -> &[Detection] {
        &self.detections
    }

    /// Record a redaction of each page-local byte `range` on its page, all or
    /// none.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] for a page the document doesn't have, or a range
    /// outside its text or off a character boundary: a redaction is never
    /// dropped, and none of the batch is recorded.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn record(
        &mut self,
        spans: impl IntoIterator<Item = (u32, Range<usize>)>,
    ) -> Result<()> {
        let mut detections = Vec::new();
        for (number, range) in spans {
            let text = self.text(number)?;
            let (Some(before), Some(_)) = (text.get(..range.start), text.get(range.clone())) else {
                return Err(Error::malformed(format!(
                    "range {}..{} is outside page {number}'s text or splits a character",
                    range.start, range.end
                )));
            };
            // An empty range covers no character, so it redacts nothing;
            // within a ligature it would otherwise select the glyph around it.
            if range.is_empty() {
                continue;
            }
            // Glyphs address characters, so the byte range is measured in
            // character offsets.
            let start = before.chars().count();
            let end = start + text[range].chars().count();
            detections.push(Detection::new(number, start, end));
        }
        self.detections.extend(detections);
        Ok(())
    }

    /// The decoded `pdf` with the recorded redactions and `plan`'s
    /// blobs applied, as bytes; the document itself when nothing changes.
    ///
    /// # Errors
    ///
    /// As [`Plan::apply`], and on the flatten path as
    /// `Plan::flatten`.
    pub(super) fn redact(&self, pdf: &Pdf, mut plan: Plan) -> Result<Bytes> {
        plan.text.clone_from(&self.detections);
        match &self.pages {
            Pages::Glyph(blocks) => {
                if plan.is_empty() {
                    return Ok(pdf.bytes());
                }
                Ok(plan.apply(pdf, blocks)?.bytes())
            }
            #[cfg(feature = "render")]
            Pages::Flatten { pages, scale } => Ok(plan.flatten(pdf, pages, *scale)?.bytes()),
        }
    }
}
