//! [`ContiguousSpan`]: join the pieces of text a mark covers into one span,
//! refusing pieces that text separates.

use std::ops::Range;

use elide_core::{Error, Result};

/// Join byte ranges of a text into the one span they cover.
///
/// A reviewer's mark can name text in several pieces: a source reference
/// each, or the glyphs under a box on a rendered page. A stream redacts a
/// mark as one span, so the pieces must form one: a span from the first to
/// the last would also take whatever text lies between them.
pub trait ContiguousSpan {
    /// The span `pieces` cover together, in any order. Pieces may overlap or
    /// touch, and may be separated by whitespace, which the span takes too.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if there are no pieces, a piece is reversed, runs
    /// past the end or splits a character, or anything but whitespace
    /// separates two pieces.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn contiguous_span<I>(&self, pieces: I) -> Result<Range<usize>>
    where
        I: IntoIterator<Item = Range<usize>>;
}

impl ContiguousSpan for str {
    fn contiguous_span<I>(&self, pieces: I) -> Result<Range<usize>>
    where
        I: IntoIterator<Item = Range<usize>>,
    {
        let mut pieces: Vec<Range<usize>> = pieces.into_iter().collect();
        for piece in &pieces {
            if self.get(piece.clone()).is_none() {
                return Err(Error::malformed(format!(
                    "range {}..{} is outside the text ({} bytes) or splits a character",
                    piece.start,
                    piece.end,
                    self.len()
                )));
            }
        }
        pieces.sort_by_key(|piece| piece.start);
        let mut pieces = pieces.into_iter();
        let mut span = pieces
            .next()
            .ok_or_else(|| Error::malformed("the mark covers no text"))?;
        for piece in pieces {
            if piece.start > span.end && !self[span.end..piece.start].trim().is_empty() {
                return Err(Error::malformed(format!(
                    "the mark covers {}..{} and {}..{}, with text between them; mark each \
                     piece on its own",
                    span.start, span.end, piece.start, piece.end
                )));
            }
            span.end = span.end.max(piece.end);
        }
        Ok(span)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    #[test]
    fn overlapping_and_touching_pieces_join() {
        let text = "call 555-0100 now";
        assert_eq!(text.contiguous_span([9..13, 5..9, 7..10]).unwrap(), 5..13);
    }

    #[test]
    fn whitespace_between_pieces_joins() {
        let text = "Jane \n Roe, Esq.";
        assert_eq!(text.contiguous_span([7..10, 0..4]).unwrap(), 0..10);
    }

    #[test]
    fn text_between_pieces_is_an_error() {
        let err = "Jane and Roe".contiguous_span([0..4, 9..12]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[test]
    fn a_bad_piece_or_none_is_an_error() {
        let text = "café";
        let err = text.contiguous_span([]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        for piece in [0..99, 0..4, Range { start: 3, end: 1 }] {
            let err = text.contiguous_span([piece.clone()]).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "{piece:?}");
        }
    }
}
