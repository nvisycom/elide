//! Opaque numeric identifiers: internal IDs, case numbers.

use fake::rand::RngExt;

use super::digits;

/// 10-digit numeric identifier shared by `InternalId` and
/// `CaseNumber` — both are opaque IDs without a globally
/// standardised format.
pub(super) fn internal_id<R: RngExt + ?Sized>(rng: &mut R) -> String {
    digits(10, rng)
}
