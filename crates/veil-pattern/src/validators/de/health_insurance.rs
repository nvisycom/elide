//! German Krankenversicherungsnummer (KVNR) checksum validator.
//!
//! 10-character KVNR: leading letter + 8 digits + check digit.
//! GKV §290 SGB V Anlage 1 expands the letter to its 2-digit
//! 1-based ordinal (A→01, B→02, …, Z→26), concatenates with the
//! 8 data digits, weights the resulting 10 digits with the
//! alternating factors `[1,2,1,2,1,2,1,2,1,2]`, cross-sums any
//! product ≥ 10, and asserts the total mod 10 equals the check
//! digit.

/// Return `true` when `value` is a valid 10-character KVNR
/// (letter + 9 digits) per GKV §290 SGB V.
pub fn health_insurance(value: &str) -> bool {
    let trimmed = value.trim().to_ascii_uppercase();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() != 10 {
        return false;
    }
    let Some(letter) = chars.first().copied() else {
        return false;
    };
    if !letter.is_ascii_uppercase() {
        return false;
    }
    if !chars[1..].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }

    // Letter → 2-digit ordinal (A=01, …, Z=26) then 8 data digits.
    let letter_val = (letter as u32) - ('A' as u32) + 1;
    let mut effective: Vec<u32> = vec![letter_val / 10, letter_val % 10];
    for c in chars.iter().skip(1).take(8) {
        effective.push(c.to_digit(10).unwrap());
    }

    let weights = [1, 2, 1, 2, 1, 2, 1, 2, 1, 2];
    let total: u32 = effective
        .iter()
        .zip(weights)
        .map(|(d, w)| {
            let p = d * w;
            if p >= 10 { (p / 10) + (p % 10) } else { p }
        })
        .sum();

    let check = chars[9].to_digit(10).unwrap();
    total % 10 == check
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_spec_example() {
        // From § 290 SGB V Anlage 1, Stand 02.01.2023.
        assert!(health_insurance("A000500015"));
    }

    #[test]
    fn rejects_wrong_check_digit() {
        assert!(!health_insurance("A000500016"));
    }

    #[test]
    fn rejects_missing_letter_prefix() {
        assert!(!health_insurance("0000500015"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!health_insurance("A00050001"));
        assert!(!health_insurance("A0005000150"));
        assert!(!health_insurance(""));
    }

    #[test]
    fn rejects_non_digit_payload() {
        assert!(!health_insurance("A00050001A"));
    }
}
