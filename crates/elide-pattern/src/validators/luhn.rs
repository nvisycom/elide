//! Luhn checksum validator for credit-card and similar identifier
//! numbers.

/// Return `true` if `num` passes the [Luhn algorithm] checksum.
///
/// Spaces and dashes are stripped before validation, so
/// `"4539 1488 0343 6467"`, `"4539-1488-0343-6467"`, and
/// `"4539148803436467"` are equivalent inputs.
///
/// Returns `false` when the input is empty or contains any
/// character other than digits, spaces, and dashes.
///
/// [Luhn algorithm]: https://en.wikipedia.org/wiki/Luhn_algorithm
pub fn luhn(num: &str) -> bool {
    if num.is_empty() {
        return false;
    }

    // Reject anything that isn't a digit, space, or dash.
    if !num
        .chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
    {
        return false;
    }

    let digits: Vec<u32> = num.chars().filter_map(|c| c.to_digit(10)).collect();

    if digits.is_empty() {
        return false;
    }

    let mut sum = 0u32;
    let mut alternate = false;
    for &n in digits.iter().rev() {
        let mut d = n;
        if alternate {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
        alternate = !alternate;
    }
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_card_numbers() {
        assert!(luhn("4539 1488 0343 6467"));
        assert!(luhn("4539148803436467"));
        assert!(luhn("4539-1488-0343-6467"));
    }

    #[test]
    fn invalid_card_numbers() {
        assert!(!luhn("4539 1488 0343 6466"));
        assert!(!luhn("1234567890123456"));
    }

    #[test]
    fn empty_input() {
        assert!(!luhn(""));
    }

    #[test]
    fn non_digit_input() {
        assert!(!luhn("abcdef"));
    }

    #[test]
    fn mixed_alpha_digit_rejected() {
        assert!(!luhn("45abc39"));
        assert!(!luhn("4539 14X8 0343 6467"));
    }

    #[test]
    fn single_zero() {
        assert!(luhn("0"));
    }

    #[test]
    fn only_separators_rejected() {
        assert!(!luhn("   "));
        assert!(!luhn("---"));
    }
}
