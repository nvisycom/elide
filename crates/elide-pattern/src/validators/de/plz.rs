//! German Postleitzahl (PLZ) validator.
//!
//! Rejects the two sentinel ranges that Deutsche Post reserves and
//! never assigns: `01000` (Briefzentrum-Sortierung test) and
//! `99999` (catch-all routing test).

const SENTINELS: &[&str] = &["01000", "99999"];

/// Return `true` when `value` is a 5-digit PLZ outside the
/// reserved sentinel ranges.
pub fn plz(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() != 5 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if trimmed.starts_with('0') && trimmed.chars().nth(1).is_none_or(|c| c == '0') {
        return false;
    }
    !SENTINELS.contains(&trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_assigned_codes() {
        assert!(plz("10117"));
        assert!(plz("01067"));
        assert!(plz("80331"));
    }

    #[test]
    fn rejects_sentinels() {
        assert!(!plz("01000"));
        assert!(!plz("99999"));
    }

    #[test]
    fn rejects_leading_zero_block() {
        assert!(!plz("00123"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!plz("1234"));
        assert!(!plz("123456"));
        assert!(!plz(""));
    }
}
