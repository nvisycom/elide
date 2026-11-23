//! [`splice`]: writing redacted items back into the source they were read
//! from.

use std::ops::Range;

use elide_core::{Error, Result};

/// One rewrite of a source: the raw range replaced, and its new raw text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    /// The source range replaced.
    pub raw: Range<usize>,
    /// The raw text written there.
    pub text: String,
}

impl Splice {
    /// `text` written over `raw`.
    pub fn new(raw: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            raw,
            text: text.into(),
        }
    }
}

/// `source` with each of `splices` written over its range, in one pass.
///
/// # Errors
///
/// [`Processing`] if a range runs past the source, splits a character, or
/// overlaps another: the items it came from were not disjoint spans of this
/// source.
///
/// [`Processing`]: elide_core::ErrorKind::Processing
pub fn splice(source: &str, mut splices: Vec<Splice>) -> Result<String> {
    splices.sort_by_key(|splice| splice.raw.start);
    let mut out = String::with_capacity(source.len());
    // The source offset copied up to.
    let mut copied = 0;
    for Splice { raw, text } in splices {
        let (Some(gap), Some(_)) = (source.get(copied..raw.start), source.get(raw.clone())) else {
            return Err(Error::processing(format!(
                "splice {}..{} is outside the source, splits a character, or overlaps \
                 another (source {} bytes)",
                raw.start,
                raw.end,
                source.len()
            )));
        };
        out.push_str(gap);
        out.push_str(&text);
        copied = raw.end;
    }
    out.push_str(&source[copied..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    #[test]
    fn splices_apply_in_any_order() {
        let splices = vec![Splice::new(8..11, "[B]"), Splice::new(0..5, "[A]")];
        assert_eq!(splice("Alice & Bob!", splices).unwrap(), "[A] & [B]!");
        assert_eq!(splice("as is", Vec::new()).unwrap(), "as is");
    }

    #[test]
    fn an_insertion_and_a_removal_splice() {
        let splices = vec![Splice::new(0..0, ">"), Splice::new(3..5, "")];
        assert_eq!(splice("abcde", splices).unwrap(), ">abc");
    }

    #[test]
    fn a_bad_range_is_refused() {
        for splices in [
            vec![Splice::new(3..9, "x")],
            vec![Splice::new(1..2, "x")],
            vec![Splice::new(0..3, "x"), Splice::new(2..4, "y")],
        ] {
            let err = splice("éabc", splices).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Processing);
        }
    }

    /// Many splices build the output in one pass.
    #[test]
    fn many_splices_are_linear() {
        let source = r#""Alice","#.repeat(100_000);
        let splices = (0..100_000)
            .map(|i| Splice::new(8 * i + 1..8 * i + 6, "X"))
            .collect();
        assert_eq!(splice(&source, splices).unwrap(), r#""X","#.repeat(100_000));
    }
}
