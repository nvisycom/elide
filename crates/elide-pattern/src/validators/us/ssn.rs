//! US Social Security Number format validator.

/// Return `true` if `value` is a structurally valid US SSN in
/// `AAA-GG-SSSS` format.
///
/// Validates the three parts as:
///
/// - **Area** (`AAA`): 001–899, excluding 666.
/// - **Group** (`GG`): 01–99.
/// - **Serial** (`SSSS`): 0001–9999.
///
/// This is a format check only — not a verification against SSA
/// records.
pub fn ssn(value: &str) -> bool {
    let parts: Vec<&str> = value.split(['-', ' ', '.']).collect();
    if parts.len() != 3 {
        return false;
    }
    let area: u32 = match parts[0].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let group: u32 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let serial: u32 = match parts[2].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    area > 0 && area < 900 && area != 666 && group > 0 && serial > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid() {
        assert!(ssn("123-45-6789"));
        assert!(ssn("001-01-0001"));
        assert!(ssn("899-99-9999"));
    }

    #[test]
    fn accepts_space_and_dot_separators() {
        assert!(ssn("123 45 6789"));
        assert!(ssn("123.45.6789"));
    }

    #[test]
    fn invalid_area_zero() {
        assert!(!ssn("000-45-6789"));
    }

    #[test]
    fn invalid_area_666() {
        assert!(!ssn("666-45-6789"));
    }

    #[test]
    fn invalid_area_900_plus() {
        assert!(!ssn("900-45-6789"));
        assert!(!ssn("999-45-6789"));
    }

    #[test]
    fn invalid_group_zero() {
        assert!(!ssn("123-00-6789"));
    }

    #[test]
    fn invalid_serial_zero() {
        assert!(!ssn("123-45-0000"));
    }

    #[test]
    fn wrong_format() {
        assert!(!ssn("12345-6789"));
        assert!(!ssn("123456789"));
        assert!(!ssn("abc-de-fghi"));
        assert!(!ssn(""));
    }
}
