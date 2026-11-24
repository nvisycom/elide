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

use elide_codec::content::ContentData;
use elide_codec::extract::OffsetMap;
use elide_codec::string::{ContiguousSpan, DisjointSpans, RedactRange};
use elide_codec::{Encode, Format, FormatId, Stream};
use elide_core::modality::text::{Marked, Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::TxtLoader;
use crate::slice::{slice, span};

/// Stable [`FormatId`] for the plain-text codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.txt");

/// The plain-text [`Format`], resolved by `.txt`/`.log` and `text/plain`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID, TxtLoader)
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
    /// The map from `text` back to the text as read, which every source
    /// reference addresses: the identity until a redaction, then one atomic
    /// run per stretch a redaction replaced.
    reads: OffsetMap,
    /// The length of the text as read.
    read_len: usize,
}

impl Stream<Text> for TxtHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        Ok(self
            .blocks
            .iter()
            .map(|range| Chunk {
                location: TextLocation::inline(range.clone()),
                data: TextData::new(self.text[range.clone()].to_string()),
                hints: Vec::new(),
            })
            .collect())
    }

    /// A source-byte mark is its own span, the source being the text: its
    /// references joined (see [`ContiguousSpan`]).
    fn resolve(&self, location: TextLocation) -> Result<TextLocation> {
        let TextLocation::Marked(mark) = &location else {
            return Ok(location);
        };
        let Marked::Source { refs } = mark else {
            return Err(Error::malformed(
                "a rendered-page mark does not address plain text",
            ));
        };
        if refs.iter().any(|source| source.part.is_some()) {
            return Err(Error::malformed(
                "a source reference names a container part, but plain text has none",
            ));
        }
        // A reference addresses the text as read; take it to where it reads
        // now, through every redaction since.
        let pieces = refs
            .iter()
            .map(|source| self.current_range(&source.range))
            .collect::<Result<Vec<_>>>()?;
        let span = self.text.contiguous_span(pieces)?;
        Ok(TextLocation::source(span, refs.clone()))
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
        Some(TextLocation::inline(start..end))
    }
}

impl Encode for TxtHandler {
    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::from_text(self.text.clone()))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for TxtHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<TextData> {
        slice(&self.text, span(location)?).map(TextData::new)
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for TxtHandler {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        // Check every span against the text before any edit, so a batch that
        // fails changes nothing. Then apply right-to-left so each edit's length
        // delta doesn't invalidate earlier locations: sort ascending by
        // position, then walk in reverse.
        let spans = redactions
            .iter()
            .map(|(location, _)| {
                let span = span(location)?;
                slice(&self.text, span)?;
                Ok(span.clone())
            })
            .collect::<Result<Vec<_>>>()?;
        spans.ensure_disjoint()?;
        redactions.sort_by_position();
        for (location, replacement) in redactions.into_iter().rev() {
            let span = span(&location)?.clone();
            let before = self.text.len();
            self.text.redact_with(&replacement, span.clone())?;
            if !span.is_empty() {
                let written = span.len() + self.text.len() - before;
                self.reads = self.reads.edited(span, written, before);
            }
        }
        // A redaction moves the text after it, and the blocks with it.
        self.blocks = paragraph_blocks(&self.text);
        Ok(())
    }
}

impl TxtHandler {
    /// A handler over the document's raw text.
    pub(crate) fn new(text: String) -> Self {
        let blocks = paragraph_blocks(&text);
        Self {
            reads: OffsetMap::identity(0, text.len()),
            read_len: text.len(),
            text,
            blocks,
        }
    }

    /// Where `read`, a range of the text as read, reads now.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] for a range the text as read does not hold.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn current_range(&self, read: &Range<usize>) -> Result<Range<usize>> {
        let ranges = self.reads.decoded_ranges(read.clone());
        match (ranges.first(), ranges.last()) {
            (Some(first), Some(last)) if read.end <= self.read_len => Ok(first.start..last.end),
            _ => Err(Error::malformed(format!(
                "source range {}..{} is outside the text ({} bytes as read)",
                read.start, read.end, self.read_len
            ))),
        }
    }

    /// The document text. Test-only inspection helper.
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
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
    use elide_core::ErrorKind;
    use elide_core::modality::text::{SourceRef, TextReplacement};
    use elide_core::primitive::Color;
    use elide_core::redaction::HiddenContentPolicy;

    use super::*;

    fn handler(text: &str) -> TxtHandler {
        TxtHandler::new(text.to_string())
    }

    fn collect_chunks(text: &str) -> Vec<(usize, usize, String)> {
        handler(text)
            .chunks()
            .unwrap()
            .into_iter()
            .map(|c| {
                let range = c.location.range().unwrap();
                (range.start, range.end, c.data.as_str().to_string())
            })
            .collect()
    }

    #[test]
    fn one_chunk_per_paragraph_block() {
        let cs = collect_chunks("para1 line1\npara1 line2\n\npara2\n");
        assert_eq!(
            cs,
            vec![
                (0, 23, "para1 line1\npara1 line2".to_string()),
                (25, 30, "para2".to_string()),
            ]
        );
    }

    #[test]
    fn a_multi_line_block_is_a_single_chunk() {
        // The PEM block spans three lines but is one chunk, so the multi-line
        // private-key pattern can match it.
        let src = "-----BEGIN KEY-----\nbody\n-----END KEY-----\n";
        let cs = collect_chunks(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].2, "-----BEGIN KEY-----\nbody\n-----END KEY-----");
    }

    #[test]
    fn blank_and_whitespace_only_lines_are_skipped() {
        let cs = collect_chunks("\n\nleading blanks\nsecond\n   \n");
        assert_eq!(cs, vec![(2, 23, "leading blanks\nsecond".to_string())]);
    }

    #[test]
    fn crlf_line_endings_are_not_carried_into_a_block() {
        // A CRLF terminator is trimmed whole, no stray `\r` at the block end,
        // while an internal CRLF between the block's lines is kept.
        let cs = collect_chunks("first\r\nsecond\r\n\r\nthird\r\n");
        assert_eq!(
            cs,
            vec![
                (0, 13, "first\r\nsecond".to_string()),
                (17, 22, "third".to_string()),
            ]
        );
    }

    #[test]
    fn no_trailing_newline() {
        let cs = collect_chunks("no newline");
        assert_eq!(cs, vec![(0, 10, "no newline".to_string())]);
    }

    #[test]
    fn lift_is_identity_across_a_line_break() {
        let h = handler("hello\nworld\n");
        let chunk = h.chunks().unwrap().into_iter().next().unwrap();
        // A span crossing the internal line break lifts unchanged, this is
        // what makes a multi-line pattern redactable.
        let lifted = h
            .lift(&chunk, TextLocation::inline(3..8))
            .expect("in bounds");
        let lifted_range = lifted.range().unwrap();
        assert_eq!(lifted_range.start, 3);
        assert_eq!(lifted_range.end, 8);
    }

    #[tokio::test]
    async fn read_returns_a_cross_line_span() -> Result<()> {
        let h = handler("hello\nworld\n");
        let loc = TextLocation::inline(3..8);
        assert_eq!(h.read_at(&loc).await?.as_str(), "lo\nwo");
        Ok(())
    }

    #[tokio::test]
    async fn redact_a_multi_line_span() -> Result<()> {
        let src = "before\n-----BEGIN KEY-----\nbody\n-----END KEY-----\nafter\n";
        let mut h = handler(src);
        let begin = src.find("-----BEGIN").unwrap();
        let end = src.find("-----END KEY-----").unwrap() + "-----END KEY-----".len();
        h.write_at(Redactions::from([(
            TextLocation::inline(begin..end),
            TextReplacement::replaced("[KEY]"),
        )]))
        .await?;
        assert_eq!(h.text(), "before\n[KEY]\nafter\n");
        Ok(())
    }

    /// A covered span is written as one mark per character it held.
    #[tokio::test]
    async fn a_covered_span_is_marked_per_character() -> Result<()> {
        let src = "Call Jané now";
        let mut h = handler(src);
        let start = src.find("Jané").unwrap();
        h.write_at(Redactions::from([(
            TextLocation::inline(start..start + "Jané".len()),
            TextReplacement::covered(Color::BLACK),
        )]))
        .await?;
        assert_eq!(h.text(), "Call ████ now");
        Ok(())
    }

    #[tokio::test]
    async fn redact_multiple_spans_any_input_order() -> Result<()> {
        let mut h = handler("alpha\nbravo\ncharlie\n");
        h.write_at(Redactions::from([
            (
                TextLocation::inline(12..19),
                TextReplacement::replaced("[C]"),
            ),
            (TextLocation::inline(0..5), TextReplacement::replaced("[A]")),
        ]))
        .await?;
        assert_eq!(h.text(), "[A]\nbravo\n[C]\n");
        Ok(())
    }

    /// A batch with a span the text can't take changes nothing, even when
    /// another of its edits, applied first, is valid.
    #[tokio::test]
    async fn a_failed_batch_leaves_the_text_as_it_was() {
        let src = "Jané Roe and Joe";
        let mut h = handler(src);
        let end = src.len();
        let err = h
            .write_at(Redactions::from([
                // Splits `é`.
                (TextLocation::inline(0..4), TextReplacement::replaced("[A]")),
                (
                    TextLocation::inline(end - 3..end),
                    TextReplacement::replaced("[B]"),
                ),
            ]))
            .await
            .expect_err("splits a character");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(h.text(), src);
    }

    /// After a write, a source reference still addresses the text as read:
    /// it finds what it named where that now reads, or what replaced it.
    #[tokio::test]
    async fn a_mark_after_a_write_addresses_the_text_as_read() -> Result<()> {
        let src = "long secret and Bob";
        let mut h = handler(src);
        h.write_at(Redactions::from([(
            TextLocation::inline(0..4),
            TextReplacement::replaced("X"),
        )]))
        .await?;
        assert_eq!(h.text(), "X secret and Bob");
        let mark = |range| TextLocation::marked_source(SourceRef::new(range));
        let bob = src.find("Bob").unwrap();
        let resolved = h.resolve(mark(bob..bob + 3))?;
        assert_eq!(h.read_at(&resolved).await?.as_str(), "Bob");
        // `ong` was replaced by `X`.
        let resolved = h.resolve(mark(1..4))?;
        assert_eq!(h.read_at(&resolved).await?.as_str(), "X");
        h.write_at(Redactions::from([(
            h.resolve(mark(bob..bob + 3))?,
            TextReplacement::replaced("[B]"),
        )]))
        .await?;
        assert_eq!(h.text(), "X secret and [B]");
        // A range the text as read never held is refused.
        assert!(h.resolve(mark(18..25)).is_err());
        Ok(())
    }

    /// After a write the blocks follow the text: each reads its paragraph as
    /// it now stands.
    #[tokio::test]
    async fn blocks_follow_a_write() -> Result<()> {
        let mut h = handler("Jane Roe\n\nnext block\n");
        h.write_at(Redactions::from([(
            TextLocation::inline(0..8),
            TextReplacement::replaced("[NAME]"),
        )]))
        .await?;
        let blocks: Vec<_> = h
            .chunks()?
            .into_iter()
            .map(|chunk| chunk.data.as_str().to_owned())
            .collect();
        assert_eq!(blocks, ["[NAME]", "next block"]);
        Ok(())
    }

    /// Each location the text can't take fails the batch rather than being
    /// skipped, which would leave the content unredacted.
    #[tokio::test]
    async fn a_location_outside_the_text_is_an_error() {
        let locations = [
            TextLocation::inline(999..1000),
            TextLocation::inline(Range { start: 4, end: 2 }),
            TextLocation::marked_source(SourceRef::new(0..3)),
        ];
        for location in locations {
            let mut h = handler("one line");
            let error = h
                .write_at(Redactions::from([(
                    location.clone(),
                    TextReplacement::replaced("nope"),
                )]))
                .await
                .expect_err("an unwritable location must fail");
            assert_eq!(error.kind(), ErrorKind::MalformedInput, "{location:?}");
            assert_eq!(h.text(), "one line");
        }
    }

    #[tokio::test]
    async fn overlapping_redactions_are_an_error() {
        let mut h = handler("Jane Roe");
        let error = h
            .write_at(Redactions::from([
                (TextLocation::inline(0..6), TextReplacement::replaced("[A]")),
                (TextLocation::inline(5..8), TextReplacement::replaced("[B]")),
            ]))
            .await
            .expect_err("overlapping redactions must fail");
        assert_eq!(error.kind(), ErrorKind::MalformedInput);
        assert_eq!(h.text(), "Jane Roe");
    }

    #[tokio::test]
    async fn a_read_the_text_cannot_take_is_an_error() {
        let h = handler("café");
        assert!(h.read_at(&TextLocation::inline(0..4)).await.is_err());
        assert!(h.read_at(&TextLocation::inline(0..99)).await.is_err());
        let mark = TextLocation::marked_source(SourceRef::new(0..3));
        assert!(h.read_at(&mark).await.is_err());
    }

    /// A source-byte mark resolves to its own span, keeping its reference.
    #[tokio::test]
    async fn a_mark_resolves_to_its_span() -> Result<()> {
        let h = handler("call 555-0100 now");
        let mark = TextLocation::marked_source(SourceRef::new(5..13));
        let resolved = h.resolve(mark)?;
        assert_eq!(resolved.range(), Some(&(5..13)));
        assert_eq!(resolved.source_refs(), [SourceRef::new(5..13)]);
        assert_eq!(h.read_at(&resolved).await?.as_str(), "555-0100");
        let past = TextLocation::marked_source(SourceRef::new(5..99));
        assert!(h.resolve(past).is_err());
        let apart = TextLocation::marked_sources([SourceRef::new(0..4), SourceRef::new(14..17)])
            .expect("refs are non-empty");
        assert!(h.resolve(apart).is_err(), "the number lies between them");
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

    /// txt decodes into a leaf [`Document`] (one `Text` stream), whose
    /// stream can be redacted in place and re-encoded.
    #[tokio::test]
    async fn leaf_document_decodes_redacts_and_encodes() -> Result<()> {
        let mut doc = format()
            .decode(
                ContentData::from_text("contact alice@example.test"),
                HiddenContentPolicy::default(),
            )
            .await?;
        assert_eq!(doc.streams_mut().len(), 1);

        // Redact the one stream in place.
        let mut stream = doc.stream_mut::<Text>("body").expect("text stream");
        // "contact " = 8 bytes; the email is 18 → 8..26.
        let batch = Redactions::from([(
            TextLocation::inline(8..26),
            TextReplacement::replaced("[EMAIL]"),
        )]);
        DataWriter::write_at(&mut stream, batch).await?;

        assert_eq!(doc.encode()?.as_bytes(), b"contact [EMAIL]");
        Ok(())
    }
}
