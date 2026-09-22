//! [`Detection`]: a span of a page's text to redact.

/// A detected span to redact: a character range into a page's text.
///
/// The offsets are Unicode scalar (`char`) offsets into the page text, the same
/// text and unit that [`Pdf::extract`](crate::Pdf::extract) and the rendered
/// observe path both produce. One `Detection` type serves every redaction path:
/// the text-layer rewrite resolves the range to glyph bytes through the page's
/// [`OffsetMap`](crate::text::OffsetMap), and the raster path resolves the same
/// range to glyph pixel boxes. A caller never picks a coordinate unit per path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    /// 1-based page number the span is on.
    pub page: u32,
    /// Start character offset into the page text (inclusive).
    pub start: usize,
    /// End character offset into the page text (exclusive).
    pub end: usize,
}

impl Detection {
    /// A detection of `[start, end)` (character offsets) on `page`.
    #[must_use]
    pub fn new(page: u32, start: usize, end: usize) -> Self {
        Self { page, start, end }
    }
}
