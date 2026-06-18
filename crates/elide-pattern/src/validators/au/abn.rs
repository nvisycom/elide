//! Australian Business Number (ABN) validator.
//!
//! 11 digits issued by the Australian Business Register (ABR).
//! Algorithm: subtract 1 from the leading digit (wrap 0→9),
//! then weighted sum with `[10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19]`
//! must be divisible by 89.

const WEIGHTS: [u32; 11] = [10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19];

/// Return `true` when `value` is a valid 11-digit ABN. Whitespace
/// and dash separators in the canonical `NN NNN NNN NNN` rendering
/// are stripped before validation.
pub fn abn(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if digits.len() != 11 || extras > 0 {
        return false;
    }
    let mut adjusted = digits;
    adjusted[0] = if adjusted[0] == 0 { 9 } else { adjusted[0] - 1 };
    let sum: u32 = adjusted.iter().zip(WEIGHTS).map(|(d, w)| d * w).sum();
    sum.is_multiple_of(89)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_abn() {
        // 51 824 753 556 — Australian Taxation Office sample.
        assert!(abn("51824753556"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(abn("51 824 753 556"));
        assert!(abn("51-824-753-556"));
    }

    #[test]
    fn accepts_second_vector() {
        // 53 004 085 616 — Telstra Corporation, public ABN.
        assert!(abn("53004085616"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!abn("11000000000"));
        assert!(!abn("51824753557"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!abn("5182475355"));
        assert!(!abn("518247535566"));
        assert!(!abn(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!abn("5182475355A"));
    }
}
