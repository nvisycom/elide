//! UK National Insurance Number prefix validator.
//!
//! Reserved-prefix exclusion lives here in the validator because
//! Rust's `regex` crate does not support look-around.

/// Return `true` when `value`'s leading two-letter prefix is not
/// a reserved NINO prefix.
///
/// Reserved prefixes (case-insensitive):
///
/// - Whole pair: `BG`, `GB`, `NK`, `KN`, `NT`, `TN`, `ZZ`.
/// - First letter `O` (HMRC reserved; not blocked by the regex
///   character class, which spans `j-p`).
///
/// The check is structural only — it does not confirm the
/// trailing suffix letter or any HMRC issuance state.
pub fn nino(value: &str) -> bool {
    let prefix: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .take(2)
        .collect();
    if prefix.len() != 2 || !prefix.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let upper = prefix.to_ascii_uppercase();
    if upper.starts_with('O') {
        return false;
    }
    !matches!(
        upper.as_str(),
        "BG" | "GB" | "NK" | "KN" | "NT" | "TN" | "ZZ"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_prefix() {
        assert!(nino("AB123456C"));
        assert!(nino("JK 12 34 56 A"));
    }

    #[test]
    fn rejects_reserved_prefixes() {
        for reserved in ["BG", "GB", "NK", "KN", "NT", "TN", "ZZ"] {
            let value = format!("{reserved}123456A");
            assert!(!nino(&value), "{reserved} must be rejected");
        }
    }

    #[test]
    fn rejection_is_case_insensitive() {
        assert!(!nino("bg123456A"));
        assert!(!nino("Zz123456A"));
    }

    #[test]
    fn rejects_non_alpha_prefix() {
        assert!(!nino("12345678A"));
        assert!(!nino(""));
    }

    #[test]
    fn rejects_o_at_position_zero() {
        assert!(!nino("OA123456A"));
        assert!(!nino("oa123456A"));
    }
}
