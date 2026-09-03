//! Plain-text handler: holds the loaded text as a single source buffer
//! and streams it as one [`Chunk<Text>`] per paragraph block, with
//! random-access reads / redactions addressed by absolute byte offset.
//!
//! A block is a maximal run of non-blank lines; blank lines delimit
//! blocks and are not themselves emitted. Chunking by block rather than
//! by line is what lets a multi-line pattern match: a PEM
//! `-----BEGIN…END-----` private-key block or a PGP block spans several
//! lines, so a per-line chunk could never contain it, while the block it
//! lives in is a single chunk. This mirrors the other codecs, which each
//! chunk by a semantic unit (CSV by cell, XML by text node, HTML by
//! block element) rather than by an arbitrary physical span.
//!
//! The buffer holds the source bytes verbatim, blank-line gaps included,
//! so a redaction splices it in place and encoding is byte-exact.

use std::ops::Range;

use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::TxtLoader;
use crate::content::ContentData;
use crate::handler::redact;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the plain-text codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.txt");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), TxtLoader)
        .with_extensions(["txt", "log"])
        .with_content_types(["text/plain"])
}

/// Handler for loaded plain-text content. The whole document is held in a
/// single buffer whose bytes are the source verbatim; `blocks` indexes the
/// paragraph spans a recognizer sees. A redaction splices the buffer in
/// place, so encoding is byte-exact and block offsets stay valid because
/// edits are applied right-to-left.
#[derive(Debug)]
pub(crate) struct TxtHandler {
    text: String,
    blocks: Vec<Range<usize>>,
    cursor: usize,
}

#[async_trait::async_trait]
impl Handler<Text> for TxtHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::from_text(self.text.clone()))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        let Some(range) = self.blocks.get(self.cursor).cloned() else {
            return Ok(None);
        };
        self.cursor += 1;
        Ok(Some(Chunk {
            location: TextLocation::new(range.start, range.end),
            data: TextData::new(self.text[range].to_string()),
            hints: Vec::new(),
        }))
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // A block chunk's bytes are a verbatim source slice, so lifting is an
        // identity offset add of the chunk-local range against the chunk's
        // start, bounded by its end.
        let chunk_range = chunk.location.range()?;
        let local_range = local.range()?;
        let base = chunk_range.start;
        let start = base + local_range.start;
        let end = base + local_range.end;
        if start > end || end > chunk_range.end {
            return None;
        }
        Some(TextLocation::new(start, end).with_page(chunk.location.page))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for TxtHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(range) = location.range() else {
            return Ok(None); // source-only location has no decoded range to read
        };
        Ok(self.text.get(range.clone()).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for TxtHandler {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        // Apply right-to-left so each edit's length delta doesn't invalidate
        // earlier locations: sort ascending by position, then walk in reverse.
        redactions.sort_by_position();
        for (location, replacement) in redactions.into_iter().rev() {
            self.redact_one(&location, &replacement)?;
        }
        Ok(())
    }
}

impl TxtHandler {
    /// Create a new handler over the document's raw text.
    pub fn new(text: String) -> Self {
        let blocks = paragraph_blocks(&text);
        Self {
            text,
            blocks,
            cursor: 0,
        }
    }

    /// The document text. Test-only inspection helper.
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    fn redact_one(&mut self, location: &TextLocation, replacement: &TextReplacement) -> Result<()> {
        // Redaction here writes by decoded byte range; a source-only location has
        // none, so it is skipped (as an unaligned range is below).
        let Some(range) = location.range() else {
            return Ok(());
        };
        let range = range.clone();
        // An inverted, out-of-bounds, or non-UTF-8-boundary range is skipped
        // rather than panicking: a detector may report a range that no longer
        // aligns after an earlier splice, and a redaction must never corrupt
        // bytes. `replace_range` panics on an inverted range, so guard it here.
        if range.start > range.end
            || range.end > self.text.len()
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return Ok(());
        }
        let value = replacement.value().unwrap_or_default();
        redact::replace_range(&mut self.text, value, range)?;
        Ok(())
    }
}

/// Byte ranges of the paragraph blocks in `text`: each is a maximal run of
/// consecutive non-blank lines. Blank (empty or whitespace-only) lines
/// delimit blocks and are excluded, as is the trailing `\n` of a block's
/// last line. A block therefore never spans a blank-line gap, so an entity
/// found inside one is contiguous in the source.
fn paragraph_blocks(text: &str) -> Vec<Range<usize>> {
    let mut blocks = Vec::new();
    let mut offset = 0usize;
    let mut current: Option<Range<usize>> = None;
    for line in text.split_inclusive('\n') {
        // Drop a trailing `\r\n` or `\n` as one line ending, so a CRLF file
        // does not carry a stray `\r` at each block's end (internal CRLF bytes
        // in a multi-line block are kept, only the terminator is trimmed).
        let ending = if line.ends_with("\r\n") {
            2
        } else {
            usize::from(line.ends_with('\n'))
        };
        let content_end = offset + line.len() - ending;
        let is_blank = text[offset..content_end].trim().is_empty();
        if is_blank {
            if let Some(block) = current.take() {
                blocks.push(block);
            }
        } else {
            match &mut current {
                Some(block) => block.end = content_end,
                None => current = Some(offset..content_end),
            }
        }
        offset += line.len();
    }
    if let Some(block) = current.take() {
        blocks.push(block);
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(text: &str) -> TxtHandler {
        TxtHandler::new(text.to_string())
    }

    async fn chunks(text: &str) -> Vec<(usize, usize, String)> {
        let mut h = handler(text);
        let mut out = Vec::new();
        while let Some(c) = h.read_next().await.unwrap() {
            let range = c.location.range().unwrap();
            out.push((range.start, range.end, c.data.as_str().to_string()));
        }
        out
    }

    #[tokio::test]
    async fn one_chunk_per_paragraph_block() {
        let cs = chunks("para1 line1\npara1 line2\n\npara2\n").await;
        assert_eq!(
            cs,
            vec![
                (0, 23, "para1 line1\npara1 line2".to_string()),
                (25, 30, "para2".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_multi_line_block_is_a_single_chunk() {
        // The PEM block spans three lines but is one chunk, so the multi-line
        // private-key pattern can match it.
        let src = "-----BEGIN KEY-----\nbody\n-----END KEY-----\n";
        let cs = chunks(src).await;
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].2, "-----BEGIN KEY-----\nbody\n-----END KEY-----");
    }

    #[tokio::test]
    async fn blank_and_whitespace_only_lines_are_skipped() {
        let cs = chunks("\n\nleading blanks\nsecond\n   \n").await;
        assert_eq!(cs, vec![(2, 23, "leading blanks\nsecond".to_string())]);
    }

    #[tokio::test]
    async fn crlf_line_endings_are_not_carried_into_a_block() {
        // A CRLF terminator is trimmed whole, no stray `\r` at the block end ,
        // while an internal CRLF between the block's lines is kept.
        let cs = chunks("first\r\nsecond\r\n\r\nthird\r\n").await;
        assert_eq!(
            cs,
            vec![
                (0, 13, "first\r\nsecond".to_string()),
                (17, 22, "third".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn no_trailing_newline() {
        let cs = chunks("no newline").await;
        assert_eq!(cs, vec![(0, 10, "no newline".to_string())]);
    }

    #[tokio::test]
    async fn lift_is_identity_across_a_line_break() {
        let mut h = handler("hello\nworld\n");
        let chunk = h.read_next().await.unwrap().unwrap();
        // A span crossing the internal line break lifts unchanged, this is
        // what makes a multi-line pattern redactable.
        let lifted = h.lift(&chunk, TextLocation::new(3, 8)).expect("in bounds");
        let lifted_range = lifted.range().unwrap();
        assert_eq!(lifted_range.start, 3);
        assert_eq!(lifted_range.end, 8);
    }

    #[tokio::test]
    async fn read_returns_a_cross_line_span() -> Result<()> {
        let h = handler("hello\nworld\n");
        let loc = TextLocation::new(3, 8);
        assert_eq!(h.read_at(&loc).await?.unwrap().as_str(), "lo\nwo");
        Ok(())
    }

    #[tokio::test]
    async fn redact_a_multi_line_span() -> Result<()> {
        let src = "before\n-----BEGIN KEY-----\nbody\n-----END KEY-----\nafter\n";
        let mut h = handler(src);
        let begin = src.find("-----BEGIN").unwrap();
        let end = src.find("-----END KEY-----").unwrap() + "-----END KEY-----".len();
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(begin, end),
            TextReplacement::substituted("[KEY]"),
        );
        h.write_at(rs).await?;
        assert_eq!(h.text(), "before\n[KEY]\nafter\n");
        Ok(())
    }

    #[tokio::test]
    async fn redact_multiple_spans_any_input_order() -> Result<()> {
        let mut h = handler("alpha\nbravo\ncharlie\n");
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(12, 19),
            TextReplacement::substituted("[C]"),
        );
        rs.push(TextLocation::new(0, 5), TextReplacement::substituted("[A]"));
        h.write_at(rs).await?;
        assert_eq!(h.text(), "[A]\nbravo\n[C]\n");
        Ok(())
    }

    #[tokio::test]
    async fn redact_unknown_location_skipped() -> Result<()> {
        let mut h = handler("one line");
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(999, 1000),
            TextReplacement::substituted("nope"),
        );
        h.write_at(rs).await?;
        assert_eq!(h.text(), "one line");
        Ok(())
    }

    #[test]
    fn encode_round_trips_with_trailing_newline() -> Result<()> {
        let h = handler("hello\nworld\n");
        assert_eq!(h.encode()?.as_bytes(), b"hello\nworld\n");
        Ok(())
    }

    #[test]
    fn encode_round_trips_without_trailing_newline() -> Result<()> {
        let h = handler("no newline");
        assert_eq!(h.encode()?.as_bytes(), b"no newline");
        Ok(())
    }

    #[test]
    fn encode_preserves_blank_line_gaps() -> Result<()> {
        // Blank lines are not chunks but must survive in the output verbatim.
        let h = handler("a\n\n\n\nb\n");
        assert_eq!(h.encode()?.as_bytes(), b"a\n\n\n\nb\n");
        Ok(())
    }
}
