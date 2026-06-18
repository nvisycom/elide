//! Turkish T.C. Kimlik No (TCKN) validator.
//!
//! 11-digit national identifier issued by Nüfus ve Vatandaşlık
//! İşleri (NVI). The first digit cannot be zero. Two check
//! digits:
//!
//! - 10th = `(sum_odd_positions * 7 - sum_even_positions) mod 10`
//!   (over the first 9 digits, 1-indexed odd/even).
//! - 11th = `sum_of_first_10 mod 10`.

/// Return `true` when `value` is a valid 11-digit TCKN.
pub fn tckn(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace())
        .count();
    if digits.len() != 11 || extras > 0 {
        return false;
    }
    if digits[0] == 0 {
        return false;
    }

    let odd_sum: i64 = (digits[0] + digits[2] + digits[4] + digits[6] + digits[8]) as i64;
    let even_sum: i64 = (digits[1] + digits[3] + digits[5] + digits[7]) as i64;
    let tenth = (odd_sum * 7 - even_sum).rem_euclid(10) as u32;
    if tenth != digits[9] {
        return false;
    }

    let eleventh: u32 = digits[..10].iter().sum::<u32>() % 10;
    eleventh == digits[10]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_tckn() {
        // Body 123456789 → 10th = 5, 11th = 0.
        assert!(tckn("12345678950"));
    }

    #[test]
    fn accepts_second_vector() {
        assert!(tckn("10000000078"));
    }

    #[test]
    fn accepts_third_vector() {
        assert!(tckn("98765432150"));
    }

    #[test]
    fn rejects_leading_zero() {
        assert!(!tckn("02345678950"));
    }

    #[test]
    fn rejects_wrong_tenth_digit() {
        assert!(!tckn("12345678900"));
    }

    #[test]
    fn rejects_wrong_eleventh_digit() {
        assert!(!tckn("12345678955"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!tckn("1234567895"));
        assert!(!tckn("123456789500"));
        assert!(!tckn(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!tckn("1234567895A"));
    }
}
