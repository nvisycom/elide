//! US ABA routing number checksum validator.

/// Return `true` if `value` is a valid 9-digit ABA RTN.
///
/// The ABA checksum sums the 9 digits with cyclic weights
/// `[3, 7, 1]` and accepts the number when the total is
/// divisible by 10.
///
/// Whitespace and `-` separators are stripped before validation,
/// so `"121000358"`, `"1210-0035-8"`, and `"121 000 358"` are
/// equivalent inputs.
pub fn aba_routing(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .map(|c| c.to_digit(10))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if digits.len() != 9 {
        return false;
    }
    let weights = [3, 7, 1, 3, 7, 1, 3, 7, 1];
    let total: u32 = digits.iter().zip(weights).map(|(d, w)| d * w).sum();
    total.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_known_numbers() {
        // Wells Fargo SF (verified test vector).
        assert!(aba_routing("121000358"));
        // JPMorgan Chase NY.
        assert!(aba_routing("021000021"));
        // Citibank NY.
        assert!(aba_routing("021000089"));
    }

    #[test]
    fn strips_separators() {
        assert!(aba_routing("121-000-358"));
        assert!(aba_routing("121 000 358"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!aba_routing("121000359"));
        assert!(!aba_routing("000000001"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!aba_routing("12100035"));
        assert!(!aba_routing("1210003580"));
        assert!(!aba_routing(""));
    }

    #[test]
    fn rejects_non_digit_payload() {
        assert!(!aba_routing("12100035A"));
    }
}
