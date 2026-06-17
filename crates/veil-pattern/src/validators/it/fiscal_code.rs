//! Italian Codice Fiscale (CF) checksum validator.
//!
//! 16 characters: 6 letters (surname/name initials) + 2 digits
//! (year) + 1 letter (month) + 2 digits (day, +40 for female) +
//! 1 letter + 3 digits (municipality code) + 1 control letter.
//!
//! Omocodia: when two people would receive the same CF, certain
//! digit positions get rewritten as specific letters
//! (`0→L, 1→M, 2→N, 3→P, 4→Q, 5→R, 6→S, 7→T, 8→U, 9→V`). The
//! checksum operates over the original alphanumeric (the regex
//! gate already allowed both forms).
//!
//! Checksum: split body (first 15 chars) into odd-indexed (1st,
//! 3rd, …) and even-indexed characters. Odd characters get a
//! lookup-table value, even characters get their plain
//! letter/digit value. Total mod 26 indexes into `A-Z` for the
//! control letter.

const ODD_TABLE: [u32; 36] = [
    // 0-9
    1, 0, 5, 7, 9, 13, 15, 17, 19, 21, // A-Z
    1, 0, 5, 7, 9, 13, 15, 17, 19, 21, 2, 4, 18, 20, 11, 3, 6, 8, 12, 14, 16, 10, 22, 25, 24, 23,
];

fn table_index(c: char) -> Option<usize> {
    match c {
        '0'..='9' => Some(c as usize - '0' as usize),
        'A'..='Z' => Some(10 + (c as usize - 'A' as usize)),
        _ => None,
    }
}

fn even_value(c: char) -> u32 {
    match c {
        '0'..='9' => c as u32 - '0' as u32,
        'A'..='Z' => c as u32 - 'A' as u32,
        _ => 0,
    }
}

/// Return `true` when `value` is a 16-character Codice Fiscale
/// whose control letter matches the computed checksum.
pub fn fiscal_code(value: &str) -> bool {
    let normalized = value.trim().to_ascii_uppercase();
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() != 16 || !chars.iter().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }

    let mut sum: u32 = 0;
    for (idx, ch) in chars[..15].iter().enumerate() {
        // CF positions are 1-indexed: even 0-index = odd
        // CF-position, so it consults ODD_TABLE.
        if idx % 2 == 0 {
            let table_idx = match table_index(*ch) {
                Some(v) => v,
                None => return false,
            };
            sum += ODD_TABLE[table_idx];
        } else {
            sum += even_value(*ch);
        }
    }

    let expected = char::from(b'A' + (sum % 26) as u8);
    chars[15] == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_cf() {
        // Surname RSS, name MRA, born 1970-01-01 in Rome (H501),
        // control letter S per Ministero delle Finanze D.M.
        // 23/12/1976 Allegato 1.
        assert!(fiscal_code("RSSMRA70A01H501S"));
    }

    #[test]
    fn accepts_second_vector() {
        // Surname MRT, name MTT, born 1925-04-09 in Florence
        // (F205), control letter Z.
        assert!(fiscal_code("MRTMTT25D09F205Z"));
    }

    #[test]
    fn accepts_lowercase_input() {
        assert!(fiscal_code("rssmra70a01h501s"));
    }

    #[test]
    fn rejects_wrong_control_letter() {
        assert!(!fiscal_code("RSSMRA70A01H501Y"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!fiscal_code("RSSMRA70A01H501"));
        assert!(!fiscal_code("RSSMRA70A01H501XX"));
        assert!(!fiscal_code(""));
    }

    #[test]
    fn rejects_non_alphanumeric() {
        assert!(!fiscal_code("RSSMRA70A01H501-"));
    }
}
