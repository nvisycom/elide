//! German Betriebsstättennummer (BSNR) structural validator.
//!
//! BSNR is a 9-digit practice/site-of-care number assigned by the
//! regional Kassenärztliche Vereinigung (KV). There is no
//! published Prüfziffer algorithm, so this validator only drops
//! obvious garbage (wrong length, non-digit, all-zero); the
//! `\b\d{9}\b` regex is too broad to promote a 2-digit prefix
//! whitelist into a high-confidence signal, so the upstream
//! `valid_kv_codes` table is left out — context keywords
//! ("Betriebsstättennummer", "Praxis", …) drive final confidence
//! via the enhancer.

/// Return `true` when `value` is a structurally-plausible BSNR.
///
/// Rejects: wrong length, non-digit characters, all-zero string.
pub fn bsnr(value: &str) -> bool {
    let digits: String = value.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if digits.len() != 9 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    !digits.chars().all(|c| c == '0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plausible_shapes() {
        assert!(bsnr("021234568"));
        assert!(bsnr("381789045"));
        assert!(bsnr("721234567"));
    }

    #[test]
    fn rejects_all_zero() {
        assert!(!bsnr("000000000"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!bsnr("12345678"));
        assert!(!bsnr("1234567890"));
        assert!(!bsnr(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!bsnr("12345678A"));
    }
}
