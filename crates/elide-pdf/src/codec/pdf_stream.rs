//! The PDF body [`Stream<Text>`]: yields each page's text as a chunk and records
//! per-page redactions onto the shared [`PdfState`].

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{FormatId, Stream};
use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::primitive::UnitBoundingBox;
use elide_core::redaction::Redactions;

use super::FORMAT_ID;
use super::pdf_state::PdfState;

/// The body [`Stream<Text>`] of a PDF: yields each page's text as a chunk and
/// records per-page redactions onto the shared [`PdfState`], which the
/// [`PdfRecombine`](super::pdf_recombine::PdfRecombine) applies on encode.
pub(super) struct PdfStream {
    pub(super) state: PdfState,
}

impl Stream<Text> for PdfStream {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        // The document re-serialisation lives in `PdfRecombine`, which reads the
        // shared state; this part's own bytes are ignored by the recombiner.
        Ok(ContentData::new(Bytes::new()))
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        // One chunk per page, each addressed in its *own* page-local byte space
        // (`0..len`). A whole-page rendered anchor carries which page the chunk
        // is on (so `lift` and redaction know it) without a document-wide offset;
        // detection runs per chunk, so a span can never straddle a page boundary.
        Ok((0..self.state.page_count())
            .filter_map(|i| self.state.page(i))
            .map(|page| Chunk {
                location: TextLocation::new(0, page.text.len())
                    .with_rects(page.number, [UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0)]),
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
        Some(TextLocation::new(local_range.start, local_range.end).with_rects(number, rects))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for PdfStream {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(range) = location.range() else {
            return Ok(None); // source-only location has no decoded range to read
        };
        // The range is page-local, addressed by the location's page number.
        let Some(number) = location.page() else {
            return Ok(None);
        };
        let Some(page) = self.state.page_by_number(number) else {
            return Ok(None);
        };
        Ok(page.text.get(range.clone()).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for PdfStream {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        for (location, _replacement) in redactions.into_iter() {
            // PDF text is addressed by a page-local decoded range plus its page;
            // a source-only location (no range) or a location with no page is
            // skipped.
            let (Some(range), Some(page)) = (location.range(), location.page()) else {
                continue;
            };
            self.state.record(page, range);
        }
        Ok(())
    }
}
