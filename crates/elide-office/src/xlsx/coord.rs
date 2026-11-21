//! [`CellCoord`]: a cell's zero-based row and column, parsed from an A1-style
//! cell reference (`B2`, `AA10`).

use std::str::FromStr;

/// The zero-based coordinates of one cell on a sheet.
///
/// Ordered row-major: by row, then by column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CellCoord {
    /// Zero-based row index.
    pub(crate) row: u32,
    /// Zero-based column index.
    pub(crate) column: u32,
}

impl CellCoord {
    /// The cell at zero-based `row` and `column`.
    pub(crate) const fn new(row: u32, column: u32) -> Self {
        Self { row, column }
    }
}

/// A cell reference that is not a run of column letters followed by a run of
/// digits addressing a row (e.g. `1A`, `A`, `A0`, an empty string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InvalidCellRef;

impl FromStr for CellCoord {
    type Err = InvalidCellRef;

    /// The cell a reference like `B2` addresses.
    ///
    /// Columns are base-26 bijective (`A`=0, `Z`=25, `AA`=26, `AB`=27); rows are
    /// the 1-based number in the reference minus one. Only ASCII `A`–`Z` and
    /// digits are accepted; a lowercase or non-ASCII byte rejects the reference.
    fn from_str(reference: &str) -> Result<Self, Self::Err> {
        parse(reference).ok_or(InvalidCellRef)
    }
}

/// The cell `reference` addresses, or `None` if it is not one.
fn parse(reference: &str) -> Option<CellCoord> {
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

    // Row: the one-based number. The split guaranteed the first byte is a
    // digit; verify the rest, rejecting trailing letters (`A1B`).
    let mut row: u32 = 0;
    for &b in digits {
        if !b.is_ascii_digit() {
            return None;
        }
        row = row.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
    }
    // Row `0` has no zero-based index, so this rejects `A0`.
    row.checked_sub(1).map(|row| CellCoord::new(row, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cell `reference` addresses, or `None` if it is not one.
    fn coord(reference: &str) -> Option<CellCoord> {
        reference.parse().ok()
    }

    #[test]
    fn parses_single_letter_columns() {
        assert_eq!(coord("A1"), Some(CellCoord::new(0, 0)));
        assert_eq!(coord("B2"), Some(CellCoord::new(1, 1)));
        assert_eq!(coord("Z9"), Some(CellCoord::new(8, 25)));
    }

    #[test]
    fn parses_multi_letter_columns() {
        assert_eq!(coord("AA1"), Some(CellCoord::new(0, 26)));
        assert_eq!(coord("AB10"), Some(CellCoord::new(9, 27)));
        assert_eq!(coord("BA1"), Some(CellCoord::new(0, 52)));
    }

    #[test]
    fn rejects_malformed_references() {
        assert_eq!(coord(""), None);
        assert_eq!(coord("A"), None); // no row
        assert_eq!(coord("1"), None); // no column
        assert_eq!(coord("1A"), None); // digits before letters
        assert_eq!(coord("A0"), None); // row 0 has no zero-based index
        assert_eq!(coord("a1"), None); // lowercase column
        assert_eq!(coord("A1B"), None); // trailing letters
    }
}
