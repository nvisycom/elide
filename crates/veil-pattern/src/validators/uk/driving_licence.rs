//! UK Driving Licence (DVLA) structural validator.
//!
//! The 16-char DVLA number opens with a 5-char surname slot —
//! letters padded on the right with `9`s when the surname is
//! shorter than five characters. A licence whose surname slot
//! is *all* `9`s, or that places a `9` before a letter (e.g.
//! `9ABCD…`, `A9BCD…`), violates the padding rule and is
//! structurally invalid.

/// Return `true` when the leading 5-char surname slot of a
/// 16-char DVLA driving licence number is structurally valid.
///
/// Rejects an all-`9` surname and any `9` that appears before a
/// letter within the slot. Does not re-validate the rest of the
/// regex-matched number — that is the regex's job.
pub fn driving_licence(value: &str) -> bool {
    let surname: Vec<char> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .take(5)
        .collect();
    if surname.len() != 5 {
        return false;
    }
    if surname.iter().all(|c| *c == '9') {
        return false;
    }
    let mut padding_started = false;
    for c in &surname {
        match c {
            '9' => padding_started = true,
            c if c.is_ascii_uppercase() => {
                if padding_started {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_padded_short_surname() {
        // 4-letter surname `MORG` padged with one `9`.
        assert!(driving_licence("MORG9753116SM9IJ"));
        // 5-letter surname `MORGA`, no padding.
        assert!(driving_licence("MORGA753116SM9IJ"));
    }

    #[test]
    fn rejects_all_nine_surname() {
        assert!(!driving_licence("99999753116SM9IJ"));
    }

    #[test]
    fn rejects_padding_before_letter() {
        // `9` precedes a letter in the surname slot.
        assert!(!driving_licence("9MORG753116SM9IJ"));
        assert!(!driving_licence("A9ORG753116SM9IJ"));
    }

    #[test]
    fn rejects_non_alpha_padding_in_surname() {
        assert!(!driving_licence("M0RGA753116SM9IJ"));
    }
}
