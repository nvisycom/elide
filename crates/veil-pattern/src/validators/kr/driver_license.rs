//! Korean driver's license number validator.
//!
//! 12 digits formatted as `AA-BB-CCCCCC-DD`. The first two
//! digits are the regional issuing office code (one of the
//! published Doro-gyotongan list). The 2-digit check digits at
//! positions 11-12 use an undisclosed algorithm, so only
//! structural + region-code validation is performed here.

const REGION_CODES: &[&str] = &[
    "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23", "24", "25", "26",
    "28",
];

/// Return `true` when `value` is a 12-digit Korean driver
/// license number with a valid region code. Hyphen and space
/// separators are stripped before validation.
pub fn driver_license(value: &str) -> bool {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if digits.len() != 12 || extras > 0 {
        return false;
    }
    REGION_CODES.contains(&&digits[..2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_license() {
        // Region 11 (Seoul) + year 20 + serial 123456 + check 78.
        assert!(driver_license("112012345678"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(driver_license("11-20-123456-78"));
        assert!(driver_license("11 20 123456 78"));
    }

    #[test]
    fn rejects_unknown_region_code() {
        // Region 99 is not in the published list.
        assert!(!driver_license("992012345678"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!driver_license("11201234567"));
        assert!(!driver_license(""));
    }
}
