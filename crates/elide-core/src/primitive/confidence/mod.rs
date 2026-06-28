//! Confidence scores and the thresholds they are compared against.
//!
//! [`Confidence`] is a *produced* score; [`ConfidenceThreshold`] is a
//! *configured* cutoff. Both are range-checked newtypes over `f32`
//! constrained to `0.0..=1.0`, kept distinct so the two cannot be
//! confused at a call site.

mod threshold;

use std::fmt;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::threshold::ConfidenceThreshold;

/// Confidence score in the closed range `0.0..=1.0`.
///
/// Carried by every provenance [`Event`] (the `before`/`after` of a
/// recognition, fusion, or calibration) and by the effective confidence
/// of an [`Entity`]. The newtype enforces the range at construction so
/// no downstream code has to defend against values outside `[0, 1]`.
///
/// Distinct from [`ConfidenceThreshold`] so the two cannot be confused
/// at a call site: a score is *produced* by detection, a threshold is a
/// *cutoff* configured to filter scores. Compare the two with
/// [`ConfidenceThreshold::passes`].
///
/// [`Event`]: crate::entity::provenance::Event
/// [`Entity`]: crate::entity::Entity
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "f32", into = "f32"))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "f32"))]
pub struct Confidence(f32);

impl Confidence {
    /// Sensible default score, `0.35`.
    ///
    /// The score to assign a detection that carries none of its own (e.g. a
    /// backend span with no model probability). Mirrors
    /// [`ConfidenceThreshold::BASELINE`]: a weak-but-plausible level that
    /// sits at the default acceptance cutoff, for a later layer to confirm
    /// or drop.
    ///
    /// [`ConfidenceThreshold::BASELINE`]: super::ConfidenceThreshold::BASELINE
    pub const BASELINE: Self = Self(0.35);
    /// Maximum score, `1.0`; full confidence.
    pub const MAX: Self = Self(1.0);
    /// Minimum score, `0.0`; no confidence.
    pub const MIN: Self = Self(0.0);

    /// Construct a score, returning [`None`] if the value is outside
    /// `0.0..=1.0` or not finite.
    pub fn new(score: f32) -> Option<Self> {
        if is_unit_interval(score) {
            Some(Self(score))
        } else {
            None
        }
    }

    /// Construct a score, clamping out-of-range values into `[0, 1]`.
    ///
    /// A non-finite input clamps to [`Confidence::MIN`].
    pub fn clamped(score: f32) -> Self {
        if score.is_nan() {
            Self::MIN
        } else {
            Self(score.clamp(0.0, 1.0))
        }
    }

    /// Score as a bare `f32`.
    pub const fn get(self) -> f32 {
        self.0
    }

    /// Add `delta` to the score, saturating at the `[0, 1]` bounds.
    ///
    /// `0.95 + 0.35` yields `1.0`, not an out-of-range value. Used by
    /// confidence-lifting steps such as context boosting.
    pub fn saturating_add(self, delta: f32) -> Self {
        Self::clamped(self.0 + delta)
    }

    /// Multiply the score by `factor`, saturating at the `[0, 1]` bounds.
    ///
    /// `0.9 * 1.2` yields `1.0`, not an out-of-range value. Used by
    /// score-scaling steps such as the `ScoreScale` backend decorator.
    pub fn saturating_mul(self, factor: f32) -> Self {
        Self::clamped(self.0 * factor)
    }
}

#[cfg(feature = "serde")]
impl TryFrom<f32> for Confidence {
    type Error = &'static str;

    fn try_from(score: f32) -> Result<Self, Self::Error> {
        Self::new(score).ok_or("confidence out of range 0.0..=1.0")
    }
}

impl From<Confidence> for f32 {
    fn from(score: Confidence) -> Self {
        score.0
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

/// Whether `value` is finite and within `0.0..=1.0`.
pub(crate) fn is_unit_interval(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
