//! The PDF body [`Stream<Text>`]: yields each page's text as a chunk and records
//! per-page redactions onto the shared [`PdfState`].

use std::ops::Range;

use elide_codec::string::ContiguousSpan;
use elide_codec::{FormatId, Stream};
use elide_core::modality::text::{Marked, Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::primitive::UnitBoundingBox;
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::FORMAT_ID;
use super::state::PdfState;

/// The body [`Stream<Text>`] of a PDF: yields each page's text as a chunk and
/// records per-page redactions onto the shared [`PdfState`], which the
/// [`PdfDocument`](super::document::PdfDocument) applies on encode.
pub(super) struct PdfStream {
    pub(super) state: PdfState,
}

impl Stream<Text> for PdfStream {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        // One chunk per page, each addressed in its *own* page-local byte space
        // (`0..len`). A whole-page rendered anchor carries which page the chunk
        // is on (so `lift` and redaction know it) without a document-wide offset;
        // detection runs per chunk, so a span can never straddle a page boundary.
        Ok(self
            .state
            .pages()
            .iter()
            .map(|page| Chunk {
                location: TextLocation::rendered(
                    0..page.text.len(),
                    page.number,
                    [UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0)],
                ),
                data: TextData::new(page.text.clone()),
                hints: Vec::new(),
            })
            .collect())
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // The chunk is already page-local, so the recognizer's range is the
        // page-local range; just validate it against the page's own bounds.
        let chunk_range = chunk.location.range()?;
        let local_range = local.range()?;
        if local_range.start > local_range.end || local_range.end > chunk_range.end {
            return None;
        }
        let number = chunk.location.page()?;

        // Resolve the entity's rendered rectangles from this page's glyph
        // geometry. The range is byte offsets, but glyph geometry is keyed by
        // character offset, so convert against the page text (which the chunk
        // carries) before looking the span up. Geometry is best-effort: a span
        // covering only synthetic gaps yields no boxes, so fall back to a
        // whole-page anchor — the entity is still located on its page.
        let page_text = chunk.data.as_str();
        let rects = self
            .state
            .geometry_for(number)
            .and_then(|geometry| {
                let char_start = page_text.get(..local_range.start)?.chars().count();
                let char_end = page_text.get(..local_range.end)?.chars().count();
                let boxes = geometry.rects_for(char_start..char_end);
                (!boxes.is_empty()).then_some(boxes)
            })
            .unwrap_or_else(|| vec![UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0)]);

        // PDF text has no flat source byte coordinate (glyph runs in content
        // streams), so no byte anchor is carried — only the page-local span and
        // the rendered rectangles.
        Some(TextLocation::rendered(local_range.clone(), number, rects))
    }

    /// A rendered-page mark becomes the page-local span of the glyphs it
    /// covers, keeping the reviewer's rectangles. The glyphs must form one
    /// span of the page text (see [`ContiguousSpan`]): a box over a table
    /// column covers glyphs with other text between them, and is refused
    /// rather than deleting that text too.
    fn resolve(&self, location: TextLocation) -> Result<TextLocation> {
        let TextLocation::Marked(mark) = &location else {
            return Ok(location);
        };
        let Marked::Rendered { page, rects } = mark else {
            return Err(Error::malformed(
                "a source-byte mark does not address a PDF page",
            ));
        };
        let number = *page;
        let text = &self
            .state
            .page_by_number(number)
            .ok_or_else(|| Error::malformed(format!("no page {number}")))?
            .text;
        let geometry = self.state.geometry_for(number).ok_or_else(|| {
            Error::malformed(format!(
                "page {number} has no glyph geometry to place a mark on"
            ))
        })?;
        // Glyphs address characters; the span addresses bytes.
        let bytes: Vec<usize> = text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
            .collect();
        let pieces = geometry
            .glyphs_in(rects)
            .map(
                |chars| match (bytes.get(chars.start), bytes.get(chars.end)) {
                    (Some(&start), Some(&end)) => Ok(start..end),
                    _ => Err(Error::malformed(format!(
                        "a glyph on page {number} is outside its text"
                    ))),
                },
            )
            .collect::<Result<Vec<_>>>()?;
        let span = text.contiguous_span(pieces)?;
        Ok(TextLocation::rendered(span, number, rects.clone()))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for PdfStream {
    async fn read_at(&self, location: &TextLocation) -> Result<TextData> {
        let (range, number) = page_span(location)?;
        let page = self
            .state
            .page_by_number(number)
            .ok_or_else(|| Error::malformed(format!("no page {number}")))?;
        page.text
            .get(range.clone())
            .map(TextData::new)
            .ok_or_else(|| {
                Error::malformed(format!(
                    "range {}..{} is outside page {number}'s text or splits a character",
                    range.start, range.end
                ))
            })
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for PdfStream {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        for (location, _replacement) in redactions.into_iter() {
            let (range, page) = page_span(&location)?;
            self.state.record(page, range)?;
        }
        Ok(())
    }
}

/// The page-local span and page a location addresses: PDF text is read and
/// written by both.
///
/// # Errors
///
/// [`MalformedInput`] for a location with no span (an unresolved mark) or no
/// page.
///
/// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
fn page_span(location: &TextLocation) -> Result<(&Range<usize>, u32)> {
    match (location.range(), location.page()) {
        (Some(range), Some(page)) => Ok((range, page)),
        _ => Err(Error::malformed(
            "a PDF location needs a page-local span and its page",
        )),
    }
}
