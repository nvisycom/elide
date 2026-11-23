//! [`PdfState`]: a decoded PDF's pages and the redactions recorded on them,
//! which the body stream owns and the document applies on encode.

use std::ops::Range;

use bytes::Bytes;
use elide_core::primitive::Color;
use elide_core::{Error, Result};

use super::plan::Plan;
use crate::content::text::{Detection, TextBlock};
use crate::pdf::Pdf;
use crate::pdf::reader::PageGeometry;

/// A redaction of a page-local byte range on a page: removed, or covered in
/// place.
pub(super) struct Span {
    pub(super) page: u32,
    pub(super) range: Range<usize>,
    pub(super) cover: Option<Color>,
}

/// A decoded PDF's text pages and the redactions recorded on them.
#[derive(Debug)]
pub(super) struct PdfState {
    /// Each page redacted by deleting its glyphs: its text with the map back
    /// to the glyphs that drew it. A page redacted from its render instead
    /// has none.
    blocks: Vec<TextBlock>,
    /// The scale the rendered pages were rendered at.
    #[cfg(feature = "render")]
    scale: f32,
    /// Recorded detections (page + character span), applied at encode.
    detections: Vec<Detection>,
}

impl PdfState {
    /// The state of the text pages `blocks`, with nothing recorded.
    pub(super) fn new(blocks: Vec<TextBlock>) -> Self {
        Self {
            blocks,
            #[cfg(feature = "render")]
            scale: super::SCAN_DPI.scale_factor(),
            detections: Vec::new(),
        }
    }

    /// The state with its rendered pages rendered at `scale`.
    #[cfg(feature = "render")]
    pub(super) fn rendered_at(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Each page's number and text, in page order, leaving out a page with no
    /// text (a scanned page, surfaced as an image instead).
    pub(super) fn texts(&self) -> Vec<(u32, &str)> {
        self.blocks
            .iter()
            .map(|block| (block.page, block.text.as_str()))
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
        self.blocks
            .iter()
            .find(|block| block.page == number)
            .map(TextBlock::geometry)
    }

    /// The scale the rendered pages were rendered at.
    #[cfg(feature = "render")]
    pub(super) fn scan_scale(&self) -> f32 {
        self.scale
    }

    /// The detections recorded so far.
    #[cfg(test)]
    pub(super) fn detections(&self) -> &[Detection] {
        &self.detections
    }

    /// Record a redaction of each of `spans`, all or none.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] for a page the document doesn't have, or a range
    /// outside its text or off a character boundary: a redaction is never
    /// dropped, and none of the batch is recorded.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn record(&mut self, spans: impl IntoIterator<Item = Span>) -> Result<()> {
        let mut detections = Vec::new();
        for Span {
            page: number,
            range,
            cover,
        } in spans
        {
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
            let detection = Detection::new(number, start..end);
            detections.push(match cover {
                Some(color) => detection.covering(color),
                None => detection,
            });
        }
        self.detections.extend(detections);
        Ok(())
    }

    /// The decoded `pdf` with the recorded redactions and `plan`'s
    /// blobs applied, as bytes; the document itself when nothing changes.
    ///
    /// # Errors
    ///
    /// As [`Plan::apply`].
    pub(super) fn redact(&self, pdf: &Pdf, mut plan: Plan) -> Result<Bytes> {
        plan.text.clone_from(&self.detections);
        if plan.is_empty() {
            return Ok(pdf.bytes());
        }
        Ok(plan.apply(pdf, &self.blocks)?.bytes())
    }
}
