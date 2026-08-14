//! [`StoredPart`]: one package part, read once and classified, with the XML
//! text extraction and splicing it supports.

use std::ops::Range;

use bytes::Bytes;
use hipstr::HipStr;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::block::{Block, IssueKind, Replacement};
use crate::error::{Error, Result};
use crate::part::{PartKind, PartPath};

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
    /// are dropped.
    pub(super) fn text_blocks(&self) -> std::result::Result<Vec<Block>, IssueKind> {
        let raw = self.as_text()?;
        let mut reader = Reader::from_str(raw);
        let mut blocks = Vec::new();
        let mut last = 0usize;

        loop {
            let event = reader.read_event().map_err(|_| IssueKind::MalformedXml)?;
            let span = last..reader.buffer_position() as usize;
            last = span.end;

            let inner = match event {
                Event::Eof => break,
                Event::Text(_) => non_blank(raw, span),
                Event::Comment(_) => strip(span, "<!--", "-->"),
                Event::CData(_) => strip(span, "<![CDATA[", "]]>"),
                _ => None,
            };
            if let Some(inner) = inner {
                blocks.push(Block {
                    part: self.path.clone(),
                    text: HipStr::from(&raw[inner.clone()]).into_owned(),
                    start: inner.start,
                    end: inner.end,
                });
            }
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

        let mut out = String::with_capacity(raw.len());
        let mut cursor = 0usize;
        for r in ordered {
            out.push_str(&raw[cursor..r.start]);
            out.push_str(&r.text);
            cursor = r.end;
        }
        out.push_str(&raw[cursor..]);
        Ok(out)
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
