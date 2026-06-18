//! German Rentenversicherungsnummer (RVNR / SVNR) validator.
//!
//! 12 characters: 8 digits + letter + 3 digits. VKVV §4
//! (Deutsche Rentenversicherung) defines the structure:
//!
//! - Pos 1–2: regional Bereichsnummer
//! - Pos 3–4: birth day (`01`–`31` or `51`–`81` with the
//!   `+50` Ergänzungsmerkmal that disambiguates duplicates)
//! - Pos 5–6: birth month (`01`–`12`)
//! - Pos 7–8: birth year (last two digits)
//! - Pos 9:   first letter of birth surname (A–Z)
//! - Pos 10–11: serial number
//! - Pos 12:  check digit (mod 10 with cross-sum)
//!
//! Checksum: expand the letter at pos 9 to its 2-digit ordinal,
//! interleave with the surrounding digits, apply weights
//! `[2,1,2,5,7,1,2,1,2,1,2,1]`, cross-sum each product (sum of
//! its two decimal digits), accept when total mod 10 == check.

/// Return `true` when `value` is a valid 12-character RVNR per
/// VKVV §4.
pub fn social_security(value: &str) -> bool {
    let trimmed = value.trim().to_ascii_uppercase();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() != 12 {
        return false;
    }
    if !chars[..8].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if !chars[8].is_ascii_uppercase() {
        return false;
    }
    if !chars[9..12].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }

    // Structural date sanity: day 01-31 or 51-81; month 01-12.
    let day: u32 = chars[2..4].iter().collect::<String>().parse().unwrap_or(0);
    let month: u32 = chars[4..6].iter().collect::<String>().parse().unwrap_or(0);
    if !((1..=31).contains(&day) || (51..=81).contains(&day)) {
        return false;
    }
    if !(1..=12).contains(&month) {
        return false;
    }

    // Letter at pos 9 → 2-digit ordinal.
    let letter_val = (chars[8] as u32) - ('A' as u32) + 1;
    let mut effective: Vec<u32> = chars[..8].iter().map(|c| c.to_digit(10).unwrap()).collect();
    effective.push(letter_val / 10);
    effective.push(letter_val % 10);
    for c in &chars[9..11] {
        effective.push(c.to_digit(10).unwrap());
    }

    let weights = [2, 1, 2, 5, 7, 1, 2, 1, 2, 1, 2, 1];
    let total: u32 = effective
        .iter()
        .zip(weights)
        .map(|(d, w)| {
            let p = d * w;
            (p / 10) + (p % 10)
        })
        .sum();

    let check = chars[11].to_digit(10).unwrap();
    total % 10 == check
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_length() {
        assert!(!social_security("12345678A12"));
        assert!(!social_security("12345678A1234"));
        assert!(!social_security(""));
    }

    #[test]
    fn rejects_invalid_day() {
        // Day 32: impossible.
        assert!(!social_security("12320178A123"));
        // Day 82: in the +50 forbidden range.
        assert!(!social_security("12820178A123"));
    }

    #[test]
    fn rejects_invalid_month() {
        // Month 13: impossible.
        assert!(!social_security("12121378A123"));
    }

    #[test]
    fn rejects_non_digit_in_serial() {
        assert!(!social_security("12010178AAAA"));
    }
}
