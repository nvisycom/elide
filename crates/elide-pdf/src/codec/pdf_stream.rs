//! The PDF body [`Stream<Text>`]: streams each page's text as a chunk and records
//! per-page redactions onto the shared [`PdfState`].

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{FormatId, Stream};
use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::FORMAT_ID;
use super::pdf_state::PdfState;

/// The body [`Stream<Text>`] of a PDF: streams each page's text as a chunk and
/// records per-page redactions onto the shared [`PdfState`], which the
/// [`PdfRecombine`](super::pdf_recombine::PdfRecombine) applies on encode.
pub(super) struct PdfStream {
    pub(super) state: PdfState,
    /// Read cursor over the pages.
    pub(super) cursor: usize,
}

#[async_trait::async_trait]
impl Stream<Text> for PdfStream {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        // The document re-serialisation lives in `PdfRecombine`, which reads the
        // shared state; this part's own bytes are ignored by the recombiner.
        Ok(ContentData::new(Bytes::new()))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        let Some(page) = self.state.page(self.cursor) else {
            return Ok(None);
        };
        let chunk = Chunk {
            location: TextLocation::new(page.start, page.start + page.text.len())
                .with_page(Some(page.number)),
            data: TextData::new(page.text.clone()),
            hints: Vec::new(),
        };
        self.cursor += 1;
        Ok(Some(chunk))
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        let chunk_range = chunk.location.range()?;
        let local_range = local.range()?;
        let base = chunk_range.start;
        let start = base + local_range.start;
        let end = base + local_range.end;
        if start > end || end > chunk_range.end {
            return None;
        }
        // PDF text has no flat source byte coordinate (glyph runs in content
        // streams), so no source range is carried.
        Some(TextLocation::new(start, end).with_page(chunk.location.page))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for PdfStream {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(range) = location.range() else {
            return Ok(None); // source-only location has no decoded range to read
        };
        let Some((page, local)) = self.state.page_at(range.start) else {
            return Ok(None);
        };
        let Some(local_end) = range.end.checked_sub(page.start) else {
            return Ok(None);
        };
        Ok(page.text.get(local..local_end).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for PdfStream {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        for (location, _replacement) in redactions.into_iter() {
            // PDF text is addressed by decoded range only; a source-only location
            // has none, so it is skipped.
            let Some(range) = location.range() else {
                continue;
            };
            self.state.record(range);
        }
        Ok(())
    }
}
