//! Plain-text handler: holds loaded text content and streams it
//! line-by-line via [`Handler<Text>`], with random-access reads /
//! redactions.
//!
//! The handler stores the text as a vector of lines together with a
//! trailing-newline flag so the original file can be reconstructed
//! byte-for-byte after edits.

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

/// Handler for loaded plain-text content. Each line is independently
/// addressable via [`TextLocation`].
///
/// `line_starts` is a cumulative-offset index maintained alongside
/// `lines`: `line_starts[i]` is the byte position of line `i` in the
/// serialized output, and `line_starts[lines.len()]` is the total-length
/// sentinel. Random-access `read_at` / `write_at` (from [`DataReader`] /
/// [`DataWriter`]) resolve a byte offset to a line in `O(log N)`.
#[derive(Debug)]
pub(crate) struct TxtHandler {
    lines: Vec<String>,
    line_starts: Vec<usize>,
    trailing_newline: bool,
    cursor: usize,
}

impl Handler<Text> for TxtHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        let mut out = self.lines.join("\n");
        if self.trailing_newline && !self.lines.is_empty() {
            out.push('\n');
        }
        Ok(ContentData::from_text(out))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        if self.cursor >= self.lines.len() {
            return Ok(None);
        }
        let i = self.cursor;
        let start = self.line_starts[i];
        let end = self.line_starts[i + 1] - 1; // strip the implicit '\n' separator
        let line = &self.lines[i];
        self.cursor += 1;
        Ok(Some(Chunk {
            location: TextLocation {
                start,
                end,
                ..Default::default()
            },
            data: TextData::new(line.clone()),
            hints: Vec::new(),
        }))
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // TXT chunks are byte-for-byte slices of source, so lifting is an
        // identity offset add of the chunk-local range against the chunk's
        // start, bounded by its end.
        let base = chunk.location.start;
        let start = base + local.start;
        let end = base + local.end;
        if start > end || end > chunk.location.end {
            return None;
        }
        Some(TextLocation {
            start,
            end,
            page: chunk.location.page,
        })
    }
}

impl DataReader<Text> for TxtHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(i) = self.line_for(location.start) else {
            return Ok(None);
        };
        let line_start = self.line_starts[i];
        let line_end = self.line_starts[i + 1] - 1;
        if location.end > line_end {
            return Ok(None); // crosses a line boundary
        }
        let local_start = location.start - line_start;
        let local_end = location.end - line_start;
        Ok(self.lines[i].get(local_start..local_end).map(TextData::new))
    }
}

impl DataWriter<Text> for TxtHandler {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        // Apply right-to-left so each edit's length delta doesn't
        // invalidate earlier locations: sort ascending by position, then
        // walk in reverse.
        redactions.sort_by_position();
        for (location, replacement) in redactions.into_iter().rev() {
            self.redact_one(&location, &replacement)?;
        }
        Ok(())
    }
}

impl TxtHandler {
    /// Create a new handler from lines and a trailing-newline flag.
    pub fn new(lines: Vec<String>, trailing_newline: bool) -> Self {
        let line_starts = compute_line_starts(&lines);
        Self {
            lines,
            line_starts,
            trailing_newline,
            cursor: 0,
        }
    }

    /// All lines in the document. Test-only inspection helper.
    #[cfg(test)]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// A specific line by 0-based index. Test-only inspection helper.
    #[cfg(test)]
    pub fn line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(String::as_str)
    }

    /// Whether the original source had a trailing newline. Test-only
    /// inspection helper.
    #[cfg(test)]
    pub fn trailing_newline(&self) -> bool {
        self.trailing_newline
    }

    /// Total number of lines. Test-only inspection helper.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Line index containing `byte_offset`, or `None` if past the end.
    fn line_for(&self, byte_offset: usize) -> Option<usize> {
        match self.line_starts.binary_search(&byte_offset) {
            Ok(i) if i < self.lines.len() => Some(i),
            Ok(_) => None, // landed on the trailing sentinel
            Err(i) if i > 0 && i <= self.lines.len() => Some(i - 1),
            _ => None,
        }
    }

    /// Shift every `line_starts[j]` for `j > i` by `delta`. Called after
    /// a redaction changes the length of line `i`.
    fn shift_starts_after(&mut self, i: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for s in &mut self.line_starts[i + 1..] {
            *s = s.saturating_add_signed(delta);
        }
    }

    fn redact_one(&mut self, location: &TextLocation, replacement: &TextReplacement) -> Result<()> {
        let Some(i) = self.line_for(location.start) else {
            return Ok(());
        };
        let line_start = self.line_starts[i];
        let line_end = self.line_starts[i + 1] - 1;
        if location.end > line_end {
            return Ok(());
        }
        let local_start = location.start - line_start;
        let local_end = location.end - line_start;
        let value = replacement.value().unwrap_or_default();
        let before_len = self.lines[i].len();
        redact::replace_range(&mut self.lines[i], value, local_start..local_end)?;
        let after_len = self.lines[i].len();
        self.shift_starts_after(i, after_len as isize - before_len as isize);
        Ok(())
    }
}

fn compute_line_starts(lines: &[String]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(lines.len() + 1);
    let mut offset = 0usize;
    for line in lines {
        starts.push(offset);
        offset += line.len() + 1; // +1 for the implicit '\n' separator
    }
    starts.push(offset);
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(text: &str) -> TxtHandler {
        let trailing_newline = text.ends_with('\n');
        let lines = text.lines().map(String::from).collect();
        TxtHandler::new(lines, trailing_newline)
    }

    #[tokio::test]
    async fn stream_yields_each_line() -> Result<()> {
        let mut h = handler("hello\nworld\n");
        let first = h.read_next().await?.unwrap();
        assert_eq!(first.location.start, 0);
        assert_eq!(first.location.end, 5);
        assert_eq!(first.data.as_str(), "hello");
        let second = h.read_next().await?.unwrap();
        assert_eq!(second.location.start, 6);
        assert_eq!(second.location.end, 11);
        assert_eq!(second.data.as_str(), "world");
        assert!(h.read_next().await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn lift_is_identity_on_second_line() -> Result<()> {
        let mut h = handler("hello\nworld\n");
        let _first = h.read_next().await?.unwrap();
        let second = h.read_next().await?.unwrap();
        let lifted = h.lift(&second, TextLocation::new(1, 4)).expect("in bounds");
        assert_eq!(lifted.start, 7);
        assert_eq!(lifted.end, 10);
        Ok(())
    }

    #[tokio::test]
    async fn read_returns_line() -> Result<()> {
        let h = handler("hello\nworld\n");
        let loc = TextLocation {
            start: 6,
            end: 11,
            ..Default::default()
        };
        assert_eq!(h.read_at(&loc).await?.unwrap().as_str(), "world");
        Ok(())
    }

    #[tokio::test]
    async fn read_cross_line_returns_none() -> Result<()> {
        let h = handler("hello\nworld\n");
        let loc = TextLocation {
            start: 3,
            end: 8,
            ..Default::default()
        };
        assert!(h.read_at(&loc).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn redact_replaces_whole_line() -> Result<()> {
        let mut h = handler("hello\nworld\n");
        let mut rs = Redactions::new();
        rs.push(
            TextLocation {
                start: 6,
                end: 11,
                ..Default::default()
            },
            TextReplacement::substituted("[REDACTED]"),
        );
        h.write_at(rs).await?;
        assert_eq!(h.lines(), &["hello", "[REDACTED]"]);
        Ok(())
    }

    #[tokio::test]
    async fn redact_multiple_lines_any_input_order() -> Result<()> {
        let mut h = handler("alpha\nbravo\ncharlie\n");
        let mut rs = Redactions::new();
        rs.push(
            TextLocation {
                start: 12,
                end: 19,
                ..Default::default()
            },
            TextReplacement::substituted("[C]"),
        );
        rs.push(
            TextLocation {
                start: 0,
                end: 5,
                ..Default::default()
            },
            TextReplacement::substituted("[A]"),
        );
        h.write_at(rs).await?;
        assert_eq!(h.lines(), &["[A]", "bravo", "[C]"]);
        Ok(())
    }

    #[tokio::test]
    async fn redact_unknown_location_skipped() -> Result<()> {
        let mut h = handler("one line");
        let mut rs = Redactions::new();
        rs.push(
            TextLocation {
                start: 999,
                end: 1000,
                ..Default::default()
            },
            TextReplacement::substituted("nope"),
        );
        h.write_at(rs).await?;
        assert_eq!(h.lines(), &["one line"]);
        Ok(())
    }

    #[test]
    fn encode_with_trailing_newline() -> Result<()> {
        let h = handler("hello\nworld\n");
        assert_eq!(h.encode()?.as_bytes(), b"hello\nworld\n");
        Ok(())
    }

    #[test]
    fn encode_without_trailing_newline() -> Result<()> {
        let h = handler("no newline");
        assert_eq!(h.encode()?.as_bytes(), b"no newline");
        Ok(())
    }

    #[tokio::test]
    async fn line_starts_shift_after_shrink() -> Result<()> {
        let mut h = handler("hello\nworld\n");
        let mut rs = Redactions::new();
        rs.push(
            TextLocation {
                start: 0,
                end: 5,
                ..Default::default()
            },
            TextReplacement::substituted("[X]"),
        );
        h.write_at(rs).await?;
        assert_eq!(h.line_starts, vec![0, 4, 10]);
        assert_eq!(h.lines(), &["[X]", "world"]);
        Ok(())
    }
}
