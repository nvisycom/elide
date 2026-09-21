//! [`Address`]: where a glyph's bytes live in the PDF's content streams.
//!
//! PDF text is not a flat byte stream: a character is drawn by a glyph whose
//! code lives inside a string operand of a content-stream operator, in a
//! specific stream (the page's own content, or a Form XObject the page draws).
//! An [`Address`] pins that location exactly, so a redaction edits the right
//! bytes of the right stream. It is the PDF analogue of a source byte span.

use lopdf::ObjectId;

/// Which content stream a glyph lives in: the page's own content, or the content
/// of a Form XObject the page (or another XObject) draws with `Do`.
///
/// A glyph's byte offsets are relative to the operand string within *this*
/// stream, and a deletion edits *this* stream. Distinguishing the streams lets
/// text drawn through a `Do`-invoked Form XObject be located and redacted like
/// page-level text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamTarget {
    /// The page's concatenated content stream.
    PageContent,
    /// A Form XObject's content stream, addressed by its object id.
    XObject(ObjectId),
}

/// The address of the string that draws a glyph: which content stream, which
/// operation within it, which operand of that operation, and which string
/// *within* that operand (an element of a `TJ` array, or the whole `Tj` string).
///
/// Ordered so it can key a `BTreeMap` of pending edits; the byte range within
/// the addressed string is carried separately (see [`OffsetRun`](super::OffsetRun)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Address {
    /// The stream the glyph's operator lives in.
    pub stream: StreamTarget,
    /// Index of the operation within the decoded content stream.
    pub op: usize,
    /// Index of the operand within that operation.
    pub operand: usize,
    /// Index of the string within a `TJ` array operand, or `None` for a plain
    /// `Tj`/`'`/`"` string operand.
    pub item: Option<usize>,
}
