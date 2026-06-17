//! Spanish CIF (Código de Identificación Fiscal) validator.
//!
//! 9 chars: entity-class letter + 7 digits + control char. The
//! control char's representation (digit vs. letter) depends on
//! the entity class — orgs starting with `P`, `Q`, `R`, `S`,
//! `N`, `W`, or `K` always carry a letter; `A`, `B`, `E`, `H`
//! always carry a digit; the rest accept either.
//!
//! Checksum:
//! 1. Double each digit at odd 1-indexed positions (positions 1,
//!    3, 5, 7 of the 7-digit body), sum the resulting decimal
//!    digits.
//! 2. Add raw digits at even 1-indexed positions (2, 4, 6).
//! 3. Compute `c = (10 - total mod 10) mod 10`.
//! 4. The control matches `c` (digit form) or
//!    `"JABCDEFGHI"[c]` (letter form).

const ENTITY_LETTERS: &str = "ABCDEFGHJNPQRSUVW";
const LETTER_ONLY: &str = "PQRSNWK";
const DIGIT_ONLY: &str = "ABEH";
const LETTER_TABLE: &[u8; 10] = b"JABCDEFGHI";

/// Return `true` when `value` is a structurally valid CIF.
pub fn cif(value: &str) -> bool {
    let normalized: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() != 9 {
        return false;
    }
    let entity = chars[0];
    if !ENTITY_LETTERS.contains(entity) {
        return false;
    }
    if !chars[1..8].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }

    let mut total: u32 = 0;
    for (idx, ch) in chars[1..8].iter().enumerate() {
        let d = ch.to_digit(10).unwrap();
        if idx % 2 == 0 {
            let doubled = d * 2;
            total += (doubled / 10) + (doubled % 10);
        } else {
            total += d;
        }
    }
    let c = (10 - total % 10) % 10;
    let control = chars[8];

    let letter_form = LETTER_TABLE[c as usize] as char;
    let digit_form = char::from(b'0' + c as u8);

    if LETTER_ONLY.contains(entity) {
        control == letter_form
    } else if DIGIT_ONLY.contains(entity) {
        control == digit_form
    } else {
        control == letter_form || control == digit_form
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_digit_form_a_class() {
        // Body 1234567: doubled-odd-positions cross-sum + raw
        // evens = 2+2+6+4+1+6+5 = 26 → c = 4. `A` is digit-only.
        assert!(cif("A12345674"));
    }

    #[test]
    fn accepts_letter_form_p_class() {
        // Same body; `P` is letter-only → LETTER_TABLE[4] = 'D'.
        assert!(cif("P1234567D"));
    }

    #[test]
    fn accepts_either_form_for_mixed_class() {
        // `C` accepts both digit and letter forms.
        assert!(cif("C12345674"));
        assert!(cif("C1234567D"));
    }

    #[test]
    fn rejects_wrong_entity_letter() {
        // `I` is not a valid CIF entity letter.
        assert!(!cif("I12345674"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!cif("A12345670"));
    }

    #[test]
    fn rejects_digit_form_for_letter_class() {
        // `P` requires a letter control; digit form must fail.
        assert!(!cif("P12345674"));
    }

    #[test]
    fn rejects_letter_form_for_digit_class() {
        // `A` requires a digit control; letter form must fail.
        assert!(!cif("A1234567D"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!cif("A1234567"));
        assert!(!cif(""));
    }
}
