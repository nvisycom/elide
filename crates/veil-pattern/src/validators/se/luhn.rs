//! Shared Luhn checksum for Swedish 10-digit identifiers.

/// Return `true` when `digits` (exactly 10 decimal digits) pass
/// the Luhn checksum used by both personnummer and
/// organisationsnummer.
pub(super) fn luhn10(digits: &[u32]) -> bool {
    let mut sum = 0u32;
    let check = digits[9];
    for (i, d) in digits[..9].iter().rev().enumerate() {
        if i.is_multiple_of(2) {
            let m = d * 2;
            sum += if m > 9 { m - 9 } else { m };
        } else {
            sum += d;
        }
    }
    (sum + check).is_multiple_of(10)
}
