//! A minimum-confidence cutoff in the closed range `0.0..=1.0`.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Confidence;

/// A minimum-confidence cutoff in the closed range `0.0..=1.0`.
///
/// A [`Confidence`] at or above the threshold *passes*; below it is
/// filtered out. Kept a separate type from [`Confidence`] so a cutoff
/// can never be passed where a score is expected, or vice versa.
///
/// [`Confidence`]: crate::primitive::Confidence
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "f32", into = "f32"))]
pub struct ConfidenceThreshold(f32);

impl ConfidenceThreshold {
    /// A sensible default cutoff, `0.35`.
    ///
    /// Mirrors Presidio's default acceptance level — low enough to
    /// retain weak-but-plausible detections for a later layer to
    /// confirm, high enough to drop near-noise.
    pub const BASELINE: Self = Self(0.35);
    /// The maximum threshold, `1.0` — accepts only full confidence.
    pub const MAX: Self = Self(1.0);
    /// The minimum threshold, `0.0` — accepts everything.
    pub const MIN: Self = Self(0.0);

    /// Construct a threshold, returning [`None`] if the value is outside
    /// `0.0..=1.0` or not finite.
    pub fn new(threshold: f32) -> Option<Self> {
        if super::is_unit_interval(threshold) {
            Some(Self(threshold))
        } else {
            None
        }
    }

    /// Whether `confidence` meets or exceeds this threshold.
    pub fn passes(self, confidence: Confidence) -> bool {
        confidence.get() >= self.0
    }

    /// The threshold as a bare `f32`.
    pub const fn get(self) -> f32 {
        self.0
    }
}

#[cfg(feature = "serde")]
impl TryFrom<f32> for ConfidenceThreshold {
    type Error = &'static str;

    fn try_from(threshold: f32) -> Result<Self, Self::Error> {
        Self::new(threshold).ok_or("confidence threshold out of range 0.0..=1.0")
    }
}

impl From<ConfidenceThreshold> for f32 {
    fn from(threshold: ConfidenceThreshold) -> Self {
        threshold.0
    }
}

impl fmt::Display for ConfidenceThreshold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}
