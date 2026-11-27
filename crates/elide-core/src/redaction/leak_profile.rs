//! [`LeakProfile`]: what an operator's output leaks about the original.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// What a redacted output leaks about the original it replaced.
///
/// It measures what survives of the value itself: its content, or its
/// extent. Where the value sat (which sentence, cell, or second of audio) is
/// not counted, since every treatment keeps its surroundings.
///
/// Variants are ordered from most-leaky to least-leaky, so `Recoverable
/// < Partial < Irrecoverable`. Surfaced through
/// [`Operator::leak_profile`] for policy authoring and audit reporting.
///
/// [`Operator::leak_profile`]: crate::redaction::Operator::leak_profile
// Explicit discriminants: the variant order is the leak ordering (`Recoverable
// < Partial < Irrecoverable`, via `Ord`, driving safest-operator arbitration)
// *and* a `Selected` event's audit hash folds `profile as u8`. Both the order
// and the values are load-bearing, so pin them, keep the ascending
// least-to-most-redacted order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LeakProfile {
    /// The value's content can be recovered from the output given the right
    /// metadata: an encryption key, the token vault or pseudonym map, or the
    /// candidate entity list against a hash.
    Recoverable = 0,
    /// Part of the value survives: some of its content (masked digits, a
    /// date's year, a bucket, a blur), or its extent (a cover's length, a
    /// box's size, a stretch of silence).
    Partial = 1,
    /// Nothing of the value survives: neither its content nor its extent.
    Irrecoverable = 2,
}
