//! Italian Partita IVA (P.IVA) checksum validator.
//!
//! 11 digits. Algorithm: split into 5 pairs of (odd, even)
//! positions (1-indexed); raw-sum the odd digits, then for each
//! even digit double it and subtract 9 when the result is ≥10,
//! sum those. Total mod 10 should equal the complement of the
//! 11th (check) digit. The all-zero string passes the math but
//! is reserved — reject explicitly.

/// Return `true` when `value` is a valid 11-digit P.IVA.
pub fn vat_code(value: &str) -> bool {
    let digits: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-' && *c != '_')
        .collect();
    let chars: Vec<char> = digits.chars().collect();
    if chars.len() != 11 || !chars.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if digits == "00000000000" {
        return false;
    }

    let mut x: u32 = 0;
    let mut y: u32 = 0;
    for i in 0..5 {
        x += chars[2 * i].to_digit(10).unwrap();
        let doubled = chars[2 * i + 1].to_digit(10).unwrap() * 2;
        y += if doubled > 9 { doubled - 9 } else { doubled };
    }
    let t = (x + y) % 10;
    let c = (10 - t) % 10;
    c == chars[10].to_digit(10).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_vat() {
        // Body 0015498056 → odd-sum 18, even-doubled-sum 13,
        // total mod 10 = 1, complement = 9.
        assert!(vat_code("00154980569"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(vat_code("00154-980-569"));
        assert!(vat_code("00 154 980 569"));
        assert!(vat_code("00_154_980_569"));
    }

    #[test]
    fn rejects_all_zero_sentinel() {
        // Passes the checksum math but is reserved.
        assert!(!vat_code("00000000000"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!vat_code("00154980560"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!vat_code("0015498056"));
        assert!(!vat_code("001549805688"));
        assert!(!vat_code(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!vat_code("0015498056A"));
    }
}
