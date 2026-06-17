//! Polish PESEL (Powszechny Elektroniczny System Ewidencji
//! Ludności) checksum validator.
//!
//! 11 digits encoding birth date + sex + serial + check digit.
//! Check digit per the Ministerstwo Cyfryzacji spec: weights
//! `[1, 3, 7, 9, 1, 3, 7, 9, 1, 3]` over the first 10 digits;
//! check = `(10 - sum mod 10) mod 10`.

/// Return `true` when `value` is a valid 11-digit PESEL.
pub fn pesel(value: &str) -> bool {
    let chars: Vec<char> = value.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if chars.len() != 11 || !chars.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let weights = [1, 3, 7, 9, 1, 3, 7, 9, 1, 3];
    let sum: u32 = chars[..10]
        .iter()
        .zip(weights)
        .map(|(c, w)| c.to_digit(10).unwrap() * w)
        .sum();
    let check = (10 - sum % 10) % 10;
    check == chars[10].to_digit(10).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_pesel() {
        // 44051401359 — widely-quoted example from official PESEL
        // documentation: born 1944-05-14, male, serial 0135,
        // check 9.
        assert!(pesel("44051401359"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!pesel("44051401350"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!pesel("4405140135"));
        assert!(!pesel("440514013599"));
        assert!(!pesel(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!pesel("4405140135A"));
    }
}
