//! Polish NIP (Numer Identyfikacji Podatkowej) validator.
//!
//! 10 digits. Check digit per Ustawa z dnia 13 października 1995
//! r. o zasadach ewidencji i identyfikacji podatników: weights
//! `[6, 5, 7, 2, 3, 4, 5, 6, 7]` over the first 9 digits;
//! check = `sum mod 11`. A computed value of 10 means the NIP is
//! invalid (never assigned).

/// Return `true` when `value` is a valid 10-digit NIP. Hyphen
/// and space separators in the conventional `XXX-XXX-XX-XX` or
/// `XXX-XX-XX-XXX` renderings are stripped before validation.
pub fn nip(value: &str) -> bool {
    let chars: Vec<char> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    if chars.len() != 10 || !chars.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let weights = [6, 5, 7, 2, 3, 4, 5, 6, 7];
    let sum: u32 = chars[..9]
        .iter()
        .zip(weights)
        .map(|(c, w)| c.to_digit(10).unwrap() * w)
        .sum();
    let check = sum % 11;
    if check == 10 {
        return false;
    }
    check == chars[9].to_digit(10).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_nip() {
        // 1060000062 — Ministerstwo Finansów reference NIP.
        assert!(nip("1060000062"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(nip("106-000-00-62"));
        assert!(nip("106 000 00 62"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!nip("1060000060"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!nip("106000006"));
        assert!(!nip("10600000622"));
        assert!(!nip(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!nip("106000006A"));
    }
}
