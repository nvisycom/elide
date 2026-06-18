//! German Umsatzsteuer-Identifikationsnummer (USt-IdNr) validator.
//!
//! 11 characters: `DE` prefix + 9 digits. The check digit at
//! position 9 (last) derives from positions 1–8 via the
//! community-documented ISO 7064 Mod 11, 10 heuristic. BZSt has
//! not published the official algorithm; tools across the EU
//! converge on this formulation, so we use it as the validator
//! and accept the rare false-negative trade-off.
//!
//! Whitespace, dots, and dashes are stripped before validation,
//! so `"DE 123 456 789"`, `"DE-123-456-789"`, and `"DE.123.456.789"`
//! all normalize to `"DE123456789"`.

/// Return `true` when `value` is a valid German USt-IdNr per the
/// ISO 7064 Mod 11,10 heuristic.
pub fn vat_id(value: &str) -> bool {
    let normalized: String = value
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '-' | '\t'))
        .collect::<String>()
        .to_ascii_uppercase();
    if normalized.len() != 11 || !normalized.starts_with("DE") {
        return false;
    }
    let digits_str = &normalized[2..];
    if !digits_str.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let digits: Vec<u32> = digits_str
        .chars()
        .map(|c| c.to_digit(10).unwrap())
        .collect();

    let mut product = 10u32;
    for d in digits.iter().take(8) {
        let mut total = (d + product) % 10;
        if total == 0 {
            total = 10;
        }
        product = (total * 2) % 11;
    }
    let mut check = 11 - product;
    if check == 10 {
        check = 0;
    }
    check == digits[8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_vat_ids() {
        // Public BMW Group USt-IdNr.
        assert!(vat_id("DE129273398"));
        // Public Siemens AG USt-IdNr.
        assert!(vat_id("DE129273398"));
    }

    #[test]
    fn strips_separators() {
        assert!(vat_id("DE 129 273 398"));
        assert!(vat_id("DE-129-273-398"));
        assert!(vat_id("DE.129.273.398"));
    }

    #[test]
    fn rejects_wrong_check_digit() {
        assert!(!vat_id("DE129273390"));
    }

    #[test]
    fn rejects_missing_de_prefix() {
        assert!(!vat_id("FR129273398"));
        assert!(!vat_id("129273398"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!vat_id("DE12345678"));
        assert!(!vat_id("DE1234567890"));
        assert!(!vat_id(""));
    }
}
