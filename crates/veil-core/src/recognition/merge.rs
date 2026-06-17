//! The [`Merge`] record — the audit entry for a fusion of entities.

use hipstr::HipStr;
use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::primitive::Confidence;

/// The record of a fusion: that several entities were combined into one.
///
/// Sits in an entity's [`Provenance`](crate::provenance::Provenance)
/// when that entity was born from fusing more than one detection. It
/// names the fusion strategy (free-text, supplied by the strategy that
/// ran), the resulting confidence, and when it happened — the
/// contributing [`Detection`](crate::recognition::Detection)s themselves
/// live alongside it in the provenance.
///
/// The *logic* of fusion (how scores combine, which span wins) lives in
/// `veil-toolkit`; this is only the audit record it writes.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Merge {
    /// Name of the fusion strategy that produced this entity.
    pub strategy: HipStr<'static>,
    /// The resulting effective confidence.
    pub confidence: Confidence,
    /// When the fusion happened (UTC).
    pub at: Timestamp,
}

impl Merge {
    /// Record a fusion event, stamped with the current time.
    pub fn new(strategy: impl Into<HipStr<'static>>, confidence: Confidence) -> Self {
        Self {
            strategy: strategy.into(),
            confidence,
            at: Timestamp::now(),
        }
    }
}
