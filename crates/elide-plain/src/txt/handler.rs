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
use elide_codec::string::{ContiguousSpan, RedactRange};
use elide_codec::{Encode, Format, FormatId, Stream};
use elide_core::modality::text::{Marked, Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::TxtLoader;
use crate::slice::slice;

/// Stable [`FormatId`] for the plain-text codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.txt");

/// The plain-text [`Format`], resolved by `.txt`/`.log` and `text/plain`.
pub fn format() -> Format {
    Format::new(FORMAT_ID, TxtLoader)
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
        let span = self
            .text
            .contiguous_span(refs.iter().map(|source| source.range.clone()))?;
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
    /// A handler over the document's raw text.
    pub(crate) fn new(text: String) -> Self {
        let blocks = paragraph_blocks(&text);
        Self { text, blocks }
    }

    /// The document text. Test-only inspection helper.
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Splice `replacement` over `location`. A location outside the text is
    /// an error: skipping it would leave the content unredacted.
    fn redact_one(&mut self, location: &TextLocation, replacement: &TextReplacement) -> Result<()> {
        let value = replacement.value().unwrap_or_default();
        self.text.redact_range(value, span(location)?.clone())
    }
}

/// The span a location addresses.
///
/// # Errors
///
/// [`MalformedInput`] for an unresolved mark, which has none.
///
/// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
fn span(location: &TextLocation) -> Result<&Range<usize>> {
    location
        .range()
        .ok_or_else(|| Error::malformed("an unresolved mark has no span"))
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
    use elide_core::modality::text::SourceRef;
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
            TextReplacement::substituted("[KEY]"),
        )]))
        .await?;
        assert_eq!(h.text(), "before\n[KEY]\nafter\n");
        Ok(())
    }

    #[tokio::test]
    async fn redact_multiple_spans_any_input_order() -> Result<()> {
        let mut h = handler("alpha\nbravo\ncharlie\n");
        h.write_at(Redactions::from([
            (
                TextLocation::inline(12..19),
                TextReplacement::substituted("[C]"),
            ),
            (
                TextLocation::inline(0..5),
                TextReplacement::substituted("[A]"),
            ),
        ]))
        .await?;
        assert_eq!(h.text(), "[A]\nbravo\n[C]\n");
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
                    TextReplacement::substituted("nope"),
                )]))
                .await
                .expect_err("an unwritable location must fail");
            assert_eq!(error.kind(), ErrorKind::MalformedInput, "{location:?}");
            assert_eq!(h.text(), "one line");
        }
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
            TextReplacement::substituted("[EMAIL]"),
        )]);
        DataWriter::write_at(&mut stream, batch).await?;

        assert_eq!(doc.encode()?.as_bytes(), b"contact [EMAIL]");
        Ok(())
    }
}
