//! [`StoredPart`]: one package part, read once and classified, with the XML
//! text extraction and splicing it supports.

use std::borrow::Cow;
use std::ops::Range;

use bytes::Bytes;
use hipstr::HipStr;
use quick_xml::Reader;
use quick_xml::escape::{partial_escape, unescape};
use quick_xml::events::Event;

use crate::block::{Block, IssueKind, OffsetMap, OffsetRun, Replacement};
use crate::error::{Error, Result};
use crate::part::{PartKind, PartPath};

/// The XML event a text span lives inside, which determines how a replacement
/// spliced into it must be escaped or framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    /// Character data of a `Text` event: escape `<`, `>`, `&`.
    Text,
    /// Body of a `<!-- ... -->` comment: reject `--` and a trailing `-`.
    Comment,
    /// Body of a `<![CDATA[ ... ]]>` section: reject `]]>`.
    Cdata,
}

/// One package part: its path, its kind, and its bytes, retained for extraction
/// and a byte-faithful re-pack.
#[derive(Debug, Clone)]
pub(super) struct StoredPart {
    path: PartPath,
    kind: PartKind,
    bytes: Bytes,
}

impl StoredPart {
    /// The part at `path` holding `bytes`, classified from its path.
    pub(super) fn new(path: PartPath, bytes: Bytes) -> Self {
        let kind = path.kind();
        Self { path, kind, bytes }
    }

    /// The part's path.
    pub(super) fn path(&self) -> &PartPath {
        &self.path
    }

    /// The part's kind.
    pub(super) fn kind(&self) -> PartKind {
        self.kind
    }

    /// The part's raw bytes (a cheap ref-counted share).
    pub(super) fn bytes(&self) -> Bytes {
        self.bytes.clone()
    }

    /// The part's bytes decoded as UTF-8, or [`IssueKind::NotUtf8`] if they are
    /// not valid UTF-8.
    fn as_text(&self) -> std::result::Result<&str, IssueKind> {
        std::str::from_utf8(&self.bytes).map_err(|_| IssueKind::NotUtf8)
    }

    /// The redactable text [`Block`]s of this (text-bearing) part, or the
    /// [`IssueKind`] that prevented extraction.
    ///
    /// Each text/comment/CDATA event's inner bytes (delimiters stripped) become
    /// a block addressed by this part and its byte span; whitespace-only runs
    /// are dropped. A block's `text` is the decoded logical text (entities like
    /// `&amp;` resolved) while its span stays raw, so splicing lands on the
    /// original bytes.
    pub(super) fn text_blocks(&self) -> std::result::Result<Vec<Block>, IssueKind> {
        let raw = self.as_text()?;
        let mut blocks = Vec::new();
        for (span, kind) in text_spans(raw).map_err(|_| IssueKind::MalformedXml)? {
            let (text, offsets): (Cow<'_, str>, OffsetMap) = match kind {
                BlockKind::Text => {
                    let decoded =
                        unescape(&raw[span.clone()]).map_err(|_| IssueKind::MalformedXml)?;
                    let offsets = offset_map(&raw[span.clone()], span.start)
                        .map_err(|_| IssueKind::MalformedXml)?;
                    (decoded, offsets)
                }
                // A comment or CDATA body is byte-identical to its raw source, so
                // the map is a single identity run over the whole span.
                BlockKind::Comment | BlockKind::Cdata => (
                    Cow::Borrowed(&raw[span.clone()]),
                    OffsetMap::identity(span.start, span.len()),
                ),
            };
            blocks.push(Block {
                part: self.path.clone(),
                text: HipStr::from(text).into_owned(),
                start: span.start,
                end: span.end,
                offsets,
            });
        }
        Ok(blocks)
    }

    /// Splice `replacements` into this part's XML, leaving every byte outside a
    /// replaced span identical. Fail-closed: validates the whole set first.
    pub(super) fn splice(&self, replacements: &[&Replacement]) -> Result<String> {
        let raw = self
            .as_text()
            .map_err(|_| Error::invalid_xml(format!("part `{}` not UTF-8", self.path)))?;
        let mut ordered = replacements.to_vec();
        ordered.sort_by_key(|r| (r.start, r.end));

        let mut prev_end = 0usize;
        for r in &ordered {
            if r.start > r.end || r.end > raw.len() {
                return Err(Error::unsafe_rewrite(format!(
                    "span {}..{} out of bounds in `{}` (len {})",
                    r.start,
                    r.end,
                    r.part,
                    raw.len()
                )));
            }
            if !raw.is_char_boundary(r.start) || !raw.is_char_boundary(r.end) {
                return Err(Error::unsafe_rewrite(format!(
                    "span {}..{} falls mid-character in `{}`",
                    r.start, r.end, r.part
                )));
            }
            if r.start < prev_end {
                return Err(Error::unsafe_rewrite(format!(
                    "span {}..{} overlaps an earlier one in `{}`",
                    r.start, r.end, r.part
                )));
            }
            prev_end = r.end;
        }

        // Recover each span's event kind so the replacement text is escaped as
        // text content, or validated against comment/CDATA framing, before it
        // enters the byte stream.
        let spans = text_spans(raw)
            .map_err(|_| Error::invalid_xml(format!("part `{}` malformed XML", self.path)))?;

        let mut out = String::with_capacity(raw.len());
        let mut cursor = 0usize;
        for r in ordered {
            let kind = span_kind(&spans, r.start, r.end).ok_or_else(|| {
                Error::unsafe_rewrite(format!(
                    "span {}..{} is not a text span in `{}`",
                    r.start, r.end, r.part
                ))
            })?;
            let safe = escape_for(kind, &r.text, r)?;
            out.push_str(&raw[cursor..r.start]);
            out.push_str(&safe);
            cursor = r.end;
        }
        out.push_str(&raw[cursor..]);
        Ok(out)
    }
}

/// Build the decoded-to-raw [`OffsetMap`] for a `Text` span whose raw bytes are
/// `slice` and whose part-absolute start is `base`.
///
/// The map interleaves identity stretches — where decoded and raw advance
/// one-for-one — with an atomic entity run per `&...;`. Each entity is unescaped
/// on its own to learn its decoded length (numeric refs `&#38;`/`&#x26;` and
/// named refs alike), which is how much of the decoded text the whole raw
/// reference stands for. Errs on an entity `unescape` rejects, matching the
/// whole-span decode.
fn offset_map(slice: &str, base: usize) -> std::result::Result<OffsetMap, ()> {
    let bytes = slice.as_bytes();
    let mut runs = Vec::new();
    let mut raw = 0usize; // slice-local raw cursor past the last entity
    let mut decoded = 0usize; // block-local decoded cursor past the last entity

    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            i += 1;
            continue;
        }
        // Find this entity's terminating `;`; a `&` with no `;` is literal text
        // (unescape leaves it untouched), so it stays inside the current run.
        let Some(semi_rel) = slice[i + 1..].find(';') else {
            i += 1;
            continue;
        };
        let end = i + 1 + semi_rel + 1;
        let token = &slice[i..end];
        let entity_decoded = unescape(token).map_err(|_| ())?;

        // Close the identity stretch up to the entity, emit the atomic entity
        // run, then step both cursors past it.
        let identity_len = i - raw;
        if identity_len > 0 {
            runs.push(OffsetRun::identity(
                decoded..decoded + identity_len,
                base + raw..base + i,
            ));
            decoded += identity_len;
        }
        runs.push(OffsetRun::entity(
            decoded..decoded + entity_decoded.len(),
            base + i..base + end,
        ));
        decoded += entity_decoded.len();
        raw = end;
        i = end;
    }

    // Flush the trailing identity stretch past the last entity.
    if raw < bytes.len() {
        let identity_len = bytes.len() - raw;
        runs.push(OffsetRun::identity(
            decoded..decoded + identity_len,
            base + raw..base + bytes.len(),
        ));
    }
    Ok(OffsetMap::new(runs))
}

/// The text/comment/CDATA spans of `raw`, each with its inner byte range
/// (delimiters stripped) and the [`BlockKind`] it belongs to; whitespace-only
/// text runs are dropped. Errs on malformed XML.
///
/// quick-xml emits a separate [`Event::GeneralRef`] for each `&entity;`, so a
/// logical text run splits into `Text`/`GeneralRef` events; a contiguous run of
/// them is coalesced into one `Text` span (covering the entity bytes) so
/// unescaping the whole span yields the decoded logical text.
fn text_spans(raw: &str) -> std::result::Result<Vec<(Range<usize>, BlockKind)>, ()> {
    let mut reader = Reader::from_str(raw);
    let mut spans = Vec::new();
    let mut last = 0usize;
    let mut run: Option<Range<usize>> = None;

    loop {
        let event = reader.read_event().map_err(|_| ())?;
        let span = last..reader.buffer_position() as usize;
        last = span.end;

        match event {
            // Extend (or open) the current text run across text and entities.
            Event::Text(_) | Event::GeneralRef(_) => {
                run = Some(run.map_or(span.clone(), |r| r.start..span.end));
                continue;
            }
            // Any other event ends the run; flush it before handling this event.
            _ => flush_text_run(raw, &mut run, &mut spans),
        }

        let found = match event {
            Event::Eof => break,
            Event::Comment(_) => strip(span, "<!--", "-->").map(|s| (s, BlockKind::Comment)),
            Event::CData(_) => strip(span, "<![CDATA[", "]]>").map(|s| (s, BlockKind::Cdata)),
            _ => None,
        };
        if let Some(found) = found {
            spans.push(found);
        }
    }
    Ok(spans)
}

/// Emit the pending text run as a `Text` span unless it is whitespace-only, then
/// clear it.
fn flush_text_run(
    raw: &str,
    run: &mut Option<Range<usize>>,
    spans: &mut Vec<(Range<usize>, BlockKind)>,
) {
    if let Some(span) = run.take()
        && let Some(span) = non_blank(raw, span)
    {
        spans.push((span, BlockKind::Text));
    }
}

/// The [`BlockKind`] of the span exactly covering `start..end`, if `start..end`
/// is one of the recorded text spans.
fn span_kind(spans: &[(Range<usize>, BlockKind)], start: usize, end: usize) -> Option<BlockKind> {
    spans
        .iter()
        .find(|(s, _)| s.start <= start && end <= s.end)
        .map(|(_, kind)| *kind)
}

/// Escape or validate `text` for splicing into a span of `kind`. Text content is
/// XML-escaped; a comment or CDATA replacement that would break its framing is a
/// fail-closed [`Error::unsafe_rewrite`].
fn escape_for<'a>(kind: BlockKind, text: &'a str, r: &Replacement) -> Result<Cow<'a, str>> {
    match kind {
        BlockKind::Text => Ok(partial_escape(text)),
        BlockKind::Comment => {
            if text.contains("--") || text.ends_with('-') {
                return Err(Error::unsafe_rewrite(format!(
                    "replacement `{}` breaks comment framing in `{}`",
                    text, r.part
                )));
            }
            Ok(Cow::Borrowed(text))
        }
        BlockKind::Cdata => {
            if text.contains("]]>") {
                return Err(Error::unsafe_rewrite(format!(
                    "replacement `{}` breaks CDATA framing in `{}`",
                    text, r.part
                )));
            }
            Ok(Cow::Borrowed(text))
        }
    }
}

/// Keep `span` unless it covers only whitespace.
fn non_blank(raw: &str, span: Range<usize>) -> Option<Range<usize>> {
    (!raw[span.clone()].trim().is_empty()).then_some(span)
}

/// Narrow `span` by its `open`/`close` delimiters to the inner range.
fn strip(span: Range<usize>, open: &str, close: &str) -> Option<Range<usize>> {
    let start = span.start.checked_add(open.len())?;
    let end = span.end.checked_sub(close.len())?;
    (start <= end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_slice_is_a_single_run() {
        let map = offset_map("Alice Bob", 0).unwrap();
        assert_eq!(map.runs(), &[OffsetRun::identity(0..9, 0..9)]);
    }

    #[test]
    fn a_named_entity_becomes_an_atomic_run_between_identity_runs() {
        // "Alice &amp; Bob" (15 raw bytes) decodes to "Alice & Bob" (11 bytes).
        let slice = "Alice &amp; Bob";
        let map = offset_map(slice, 0).unwrap();
        assert_eq!(
            map.runs(),
            &[
                OffsetRun::identity(0..6, 0..6),
                OffsetRun::entity(6..7, 6..11),
                OffsetRun::identity(7..11, 11..15),
            ]
        );
        // The whole decoded text maps back to the full raw slice incl. `&amp;`,
        // as one contiguous range.
        assert_eq!(map.raw_ranges(0..11), vec![0..15]);
        // A base offset is folded straight into the raw ranges.
        let based = offset_map(slice, 100).unwrap();
        assert_eq!(based.raw_ranges(0..11), vec![100..115]);
    }

    #[test]
    fn numeric_and_hex_char_refs_map_like_named_ones() {
        // `&#38;` and `&#x26;` both decode to `&` (1 byte) but are 5 / 6 raw
        // bytes; the whole decoded text maps back to the full raw slice.
        let dec = offset_map("A&#38;B", 0).unwrap();
        assert_eq!(dec.raw_ranges(0..3), vec![0..7]);
        // Just the decoded `&` (offset 1) maps to the entity's raw bytes.
        assert_eq!(dec.raw_ranges(1..2), vec![1..6]);
        let hex = offset_map("A&#x26;B", 0).unwrap();
        assert_eq!(hex.raw_ranges(0..3), vec![0..8]);
        assert_eq!(hex.raw_ranges(1..2), vec![1..7]);
    }

    #[test]
    fn a_bare_ampersand_without_semicolon_stays_in_one_run() {
        // Not an entity reference, so decoded and raw stay identical throughout.
        let map = offset_map("a & b", 0).unwrap();
        assert_eq!(map.runs(), &[OffsetRun::identity(0..5, 0..5)]);
    }

    #[test]
    fn a_range_crossing_one_entity_pulls_its_whole_raw() {
        // Decoded "e & B" straddles the entity in "Alice &amp; Bob".
        let map = offset_map("Alice &amp; Bob", 0).unwrap();
        // decoded "e"(4) .. " B"(9) → identity tail [4..6] + whole `&amp;`
        // [6..11] + identity head [11..13], all contiguous → one range.
        assert_eq!(map.raw_ranges(4..9), vec![4..13]);
    }
}
