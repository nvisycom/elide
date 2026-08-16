//! Parsing an A1-style cell reference (`B2`, `AA10`) into zero-based row and
//! column indices.

/// The zero-based `(row, column)` a cell reference like `B2` addresses, or
/// `None` if the reference is not a run of column letters followed by a run of
/// digits (e.g. `1A`, `A`, `A0`, an empty string).
///
/// Columns are base-26 bijective (`A`=0, `Z`=25, `AA`=26, `AB`=27); rows are the
/// 1-based number in the reference minus one. Only ASCII `A`–`Z` and digits are
/// accepted; a lowercase or non-ASCII byte rejects the reference.
pub(crate) fn parse_cell_ref(reference: &str) -> Option<(u32, u32)> {
    let bytes = reference.as_bytes();
    let split = bytes.iter().position(u8::is_ascii_digit)?;
    let (letters, digits) = (&bytes[..split], &bytes[split..]);
    if letters.is_empty() || digits.is_empty() {
        return None;
    }

    // Column: bijective base-26 over A–Z. `A`=1 … `Z`=26, `AA`=27 in one-based
    // terms, so the running value is one greater than the zero-based index.
    let mut column: u32 = 0;
    for &b in letters {
        if !b.is_ascii_uppercase() {
            return None;
        }
        column = column
            .checked_mul(26)?
            .checked_add(u32::from(b - b'A') + 1)?;
    }
    let column = column.checked_sub(1)?;

    // Row: the one-based number, minus one. Every remaining byte is a digit
    // (the split guaranteed the first is; verify the rest and reject `A0`).
    let mut row: u32 = 0;
    for &b in digits {
        if !b.is_ascii_digit() {
            return None;
        }
        row = row.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
    }
    row.checked_sub(1).map(|row| (row, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_letter_columns() {
        assert_eq!(parse_cell_ref("A1"), Some((0, 0)));
        assert_eq!(parse_cell_ref("B2"), Some((1, 1)));
        assert_eq!(parse_cell_ref("Z9"), Some((8, 25)));
    }

    #[test]
    fn parses_multi_letter_columns() {
        assert_eq!(parse_cell_ref("AA1"), Some((0, 26)));
        assert_eq!(parse_cell_ref("AB10"), Some((9, 27)));
        assert_eq!(parse_cell_ref("BA1"), Some((0, 52)));
    }

    #[test]
    fn rejects_malformed_references() {
        assert_eq!(parse_cell_ref(""), None);
        assert_eq!(parse_cell_ref("A"), None); // no row
        assert_eq!(parse_cell_ref("1"), None); // no column
        assert_eq!(parse_cell_ref("1A"), None); // digits before letters
        assert_eq!(parse_cell_ref("A0"), None); // row 0 has no zero-based index
        assert_eq!(parse_cell_ref("a1"), None); // lowercase column
        assert_eq!(parse_cell_ref("A1B"), None); // trailing letters
    }
}
