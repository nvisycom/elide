//! [`StoredPart`]: one package part, read once and classified, with the XML
//! text extraction and splicing it supports.

use std::borrow::Cow;
use std::ops::Range;

use bytes::Bytes;
use hipstr::HipStr;
use quick_xml::Reader;
use quick_xml::escape::{partial_escape, unescape};
use quick_xml::events::Event;

use crate::block::{Block, IssueKind, Replacement};
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
            let text: Cow<'_, str> = match kind {
                BlockKind::Text => {
                    unescape(&raw[span.clone()]).map_err(|_| IssueKind::MalformedXml)?
                }
                BlockKind::Comment | BlockKind::Cdata => Cow::Borrowed(&raw[span.clone()]),
            };
            blocks.push(Block {
                part: self.path.clone(),
                text: HipStr::from(text).into_owned(),
                start: span.start,
                end: span.end,
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
