//! [`Multiplier`]: a checked confidence multiplier.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A confidence multiplier: finite and non-negative.
///
/// A negative or NaN multiplier has no meaning as a scale, and an infinite one
/// turns a zero score into NaN, so neither can be built. A large finite
/// multiplier is allowed: scaled confidence clamps to `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "f64", into = "f64"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "f64"))]
pub struct Multiplier(f64);

impl Multiplier {
    /// The multiplier that leaves a confidence as it is.
    pub const IDENTITY: Self = Self(1.0);

    /// `factor` as a multiplier, or [`None`] when it is negative or not
    /// finite.
    ///
    /// A `const fn`, so a literal is checked when the program compiles:
    ///
    /// ```
    /// use elide_detection::decorator::calibrated::Multiplier;
    ///
    /// const REGEX: Multiplier = Multiplier::new(0.8).unwrap();
    /// assert_eq!(REGEX.get(), 0.8);
    /// ```
    ///
    /// ```compile_fail
    /// use elide_detection::decorator::calibrated::Multiplier;
    ///
    /// const NEGATIVE: Multiplier = Multiplier::new(-0.8).unwrap();
    /// ```
    pub const fn new(factor: f64) -> Option<Self> {
        if factor.is_finite() && factor >= 0.0 {
            Some(Self(factor))
        } else {
            None
        }
    }

    /// The factor.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Default for Multiplier {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(feature = "serde")]
impl TryFrom<f64> for Multiplier {
    type Error = &'static str;

    fn try_from(factor: f64) -> Result<Self, Self::Error> {
        Self::new(factor).ok_or("calibration multiplier must be finite and non-negative")
    }
}

impl From<Multiplier> for f64 {
    fn from(multiplier: Multiplier) -> Self {
        multiplier.0
    }
}

impl fmt::Display for Multiplier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_or_non_finite_multiplier_cannot_be_made() {
        for bad in [-0.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(Multiplier::new(bad).is_none());
        }
        assert_eq!(Multiplier::new(0.0).map(Multiplier::get), Some(0.0));
        assert_eq!(Multiplier::new(3.0).map(Multiplier::get), Some(3.0));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserializing_checks_the_factor() {
        assert_eq!(
            serde_json::from_str::<Multiplier>("0.5").unwrap().get(),
            0.5
        );
        assert!(serde_json::from_str::<Multiplier>("-1.0").is_err());
    }
}
