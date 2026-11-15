//! [`DisjointSpans`]: refuse a batch of redactions whose spans overlap.

use std::ops::Range;

use elide_core::{Error, Result};

/// Check that a batch's byte ranges do not overlap.
///
/// A text stream applies a batch right to left, so an edit never moves the
/// ones still to come. Two edits over the same bytes break that: the second
/// would splice into the first's replacement. The anonymizer merges
/// overlapping findings, so only a batch built by hand has them.
pub trait DisjointSpans {
    /// Whether no two spans overlap. Spans that touch are disjoint.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] naming an overlapping pair.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn ensure_disjoint(&self) -> Result<()>;
}

impl DisjointSpans for [Range<usize>] {
    fn ensure_disjoint(&self) -> Result<()> {
        let mut spans: Vec<&Range<usize>> = self.iter().collect();
        spans.sort_by_key(|span| span.start);
        match spans.windows(2).find(|pair| pair[0].end > pair[1].start) {
            Some(pair) => Err(Error::malformed(format!(
                "redactions {}..{} and {}..{} overlap; merge them into one",
                pair[0].start, pair[0].end, pair[1].start, pair[1].end
            ))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    #[test]
    fn touching_spans_are_disjoint() {
        let spans = [5..8, 0..5, 8..8];
        assert!(spans.ensure_disjoint().is_ok());
    }

    #[test]
    fn overlapping_spans_of_one_key_are_an_error() {
        for spans in [[0..6, 5..8], [3..4, 0..8]] {
            let err = spans.ensure_disjoint().unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "{spans:?}");
        }
    }
}
