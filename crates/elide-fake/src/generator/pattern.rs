//! Pattern-preserving reshape: emit a string whose length and
//! character-class layout matches `original` exactly. Digits in
//! `original` map to fresh random digits in the output; letters
//! map to fresh random letters of matching case; everything else
//! is copied verbatim.
//!
//! Used by structured kinds (IBAN, payment cards, postal codes,
//! phone numbers, dates) where the shape carries meaning. Free-form
//! kinds (names, addresses) don't go through this — their output
//! comes straight from the locale-aware generator.

use fake::Fake;
use fake::rand::RngExt;

/// Walk `original` left-to-right and emit a same-length output:
/// - digit `→` random ASCII digit `0..=9`
/// - uppercase ASCII letter `→` random uppercase `A..=Z`
/// - lowercase ASCII letter `→` random lowercase `a..=z`
/// - other code-point (separators, multibyte, punctuation) `→`
///   copied unchanged
pub(crate) fn pattern_preserve<R: RngExt + ?Sized>(original: &str, rng: &mut R) -> String {
    const DIGITS: &[u8; 10] = b"0123456789";
    const UPPER: &[u8; 26] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const LOWER: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";

    let mut out = String::with_capacity(original.len());
    for ch in original.chars() {
        if ch.is_ascii_digit() {
            let i: usize = (0..DIGITS.len()).fake_with_rng(rng);
            out.push(DIGITS[i] as char);
        } else if ch.is_ascii_uppercase() {
            let i: usize = (0..UPPER.len()).fake_with_rng(rng);
            out.push(UPPER[i] as char);
        } else if ch.is_ascii_lowercase() {
            let i: usize = (0..LOWER.len()).fake_with_rng(rng);
            out.push(LOWER[i] as char);
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use fake::rand::SeedableRng;
    use fake::rand::rngs::SmallRng;

    use super::*;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(7)
    }

    #[test]
    fn preserves_iban_shape() {
        let out = pattern_preserve("GB82WEST12345698765432", &mut rng());
        assert_eq!(out.len(), 22);
        // Char-class layout: 2 upper + 2 digit + 4 upper + 14 digit
        assert!(out.as_bytes()[0].is_ascii_uppercase());
        assert!(out.as_bytes()[1].is_ascii_uppercase());
        assert!(out.as_bytes()[2].is_ascii_digit());
        assert!(out.as_bytes()[3].is_ascii_digit());
        for i in 4..8 {
            assert!(out.as_bytes()[i].is_ascii_uppercase(), "pos {i}");
        }
        for i in 8..22 {
            assert!(out.as_bytes()[i].is_ascii_digit(), "pos {i}");
        }
    }

    #[test]
    fn preserves_phone_separators() {
        let out = pattern_preserve("555-123-4567", &mut rng());
        assert_eq!(out.len(), 12);
        assert_eq!(&out[3..4], "-");
        assert_eq!(&out[7..8], "-");
    }

    #[test]
    fn preserves_cjk_date_separators() {
        let out = pattern_preserve("1985年3月12日", &mut rng());
        assert!(out.contains('年') && out.contains('月') && out.contains('日'));
    }

    #[test]
    fn preserves_us_date_format() {
        let out = pattern_preserve("03/14/2024", &mut rng());
        assert_eq!(out.len(), 10);
        assert_eq!(&out[2..3], "/");
        assert_eq!(&out[5..6], "/");
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert_eq!(pattern_preserve("", &mut rng()), "");
    }
}
