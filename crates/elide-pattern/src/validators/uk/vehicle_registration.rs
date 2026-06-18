//! UK current-format Vehicle Registration Mark (VRM) age-ID
//! validator.
//!
//! Current (2001+) plates encode the issuance half-year as a
//! 2-digit "age identifier" at positions 3-4:
//!
//! - March issue: `02..=29` (March 2002 through March 2029)
//! - September issue: `51..=79` (September 2001 through September
//!   2029)
//!
//! The recognizer regex permits the broader range `01..=79`
//! (cheap to express); this validator narrows it to the issued
//! windows that the DVLA actually allocates.

/// Return `true` when the 2-digit age identifier embedded in a
/// 7-char current-format UK plate falls inside an issued range.
///
/// Strips whitespace and `-`, then reads characters at positions
/// 2 and 3 of the canonicalized string.
pub fn vehicle_registration(value: &str) -> bool {
    let chars: Vec<char> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    if chars.len() != 7 {
        return false;
    }
    let age = match (chars[2].to_digit(10), chars[3].to_digit(10)) {
        (Some(a), Some(b)) => a * 10 + b,
        _ => return false,
    };
    matches!(age, 2..=29 | 51..=79)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_march_window() {
        assert!(vehicle_registration("AB02ABC"));
        assert!(vehicle_registration("AB29ABC"));
        assert!(vehicle_registration("AB 15 ABC"));
    }

    #[test]
    fn accepts_september_window() {
        assert!(vehicle_registration("AB51ABC"));
        assert!(vehicle_registration("AB79ABC"));
        assert!(vehicle_registration("AB-65-ABC"));
    }

    #[test]
    fn rejects_out_of_range() {
        // 01 was used briefly in 2001 but is not in the modern
        // issued range; presidio rejects it too.
        assert!(!vehicle_registration("AB01ABC"));
        assert!(!vehicle_registration("AB30ABC"));
        assert!(!vehicle_registration("AB50ABC"));
        assert!(!vehicle_registration("AB80ABC"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!vehicle_registration("AB51AB"));
        assert!(!vehicle_registration("AB51ABCD"));
        assert!(!vehicle_registration(""));
    }
}
