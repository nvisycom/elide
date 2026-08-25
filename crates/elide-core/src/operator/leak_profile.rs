//! [`LeakProfile`]: what an operator's output leaks about the original.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// What a redacted output leaks about the original it replaced.
///
/// Variants are ordered from most-leaky to least-leaky, so `Recoverable
/// < Partial < Irrecoverable`. Surfaced through
/// [`Operator::leak_profile`] for policy authoring and audit reporting.
///
/// [`Operator::leak_profile`]: crate::operator::Operator::leak_profile
// Explicit discriminants: the variant order is the leak ordering (`Recoverable
// < Partial < Irrecoverable`, via `Ord`, driving safest-operator arbitration)
// *and* the `Redaction` audit hash folds `profile as u8`. Both the order and
// the values are load-bearing, so pin them — keep the ascending
// least-to-most-redacted order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LeakProfile {
    /// The original value is recoverable from the output given the right
    /// metadata (encryption key, token vault, pseudonym map, or the
    /// candidate entity list against a hash).
    Recoverable = 0,
    /// The original value is gone, but observable shape leaks: position,
    /// length, bounding box, cell coordinates, or a known silence on the
    /// timeline.
    Partial = 1,
    /// No trace of the original value or its shape remains in the output.
    Irrecoverable = 2,
}
