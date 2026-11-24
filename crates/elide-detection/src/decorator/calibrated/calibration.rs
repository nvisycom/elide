//! [`Calibration`]: how a [`CalibratedRecognizer`](super::CalibratedRecognizer)
//! rescales its recognizer's confidence.

use std::collections::HashMap;

use elide_core::entity::LabelRef;
use elide_core::{Error, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A recognizer's confidence multipliers: one for every label, and others for
/// the labels whose scores run differently.
///
/// Compensates for score shapes that differ between recognizers, so their
/// findings compare fairly when overlaps are fused and arbitrated: a regex
/// that always scores `1.0` and a NER model that scores `0.3–0.9` can be
/// brought into one range (a `0.8` on the regex). A model whose scores run
/// differently per label (confident on names, overconfident on organizations)
/// takes a multiplier per label on top.
///
/// A multiplier is finite and non-negative, however the calibration is built
/// (the constructors or deserialized): a negative or NaN multiplier has no
/// meaning as a scale, and an infinite one turns a zero score into NaN. A large
/// finite multiplier is allowed: scaled confidence clamps to `[0, 1]`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(try_from = "Multipliers", into = "Multipliers")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "Multipliers"))]
pub struct Calibration(Multipliers);

/// A [`Calibration`]'s multipliers, as written.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct Multipliers {
    /// The multiplier for a label with none of its own.
    #[cfg_attr(feature = "serde", serde(default = "unit"))]
    scale: f64,
    /// The multipliers for labels whose scores run differently.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    labels: HashMap<LabelRef, f64>,
}

#[cfg(feature = "serde")]
fn unit() -> f64 {
    1.0
}

impl Calibration {
    /// Scale every label's confidence by `factor`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Configuration`](elide_core::ErrorKind::Configuration) if
    /// `factor` is negative or not finite.
    pub fn scale(factor: f64) -> Result<Self> {
        Ok(Self(Multipliers {
            scale: checked("every label", factor)?,
            labels: HashMap::new(),
        }))
    }

    /// Scale `label`'s confidence by `factor` instead.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Configuration`](elide_core::ErrorKind::Configuration) if
    /// `factor` is negative or not finite.
    pub fn with_label(mut self, label: impl Into<LabelRef>, factor: f64) -> Result<Self> {
        let label = label.into();
        let factor = checked(label.as_str(), factor)?;
        self.0.labels.insert(label, factor);
        Ok(self)
    }

    /// The multiplier for `label`: its own, or the one for every label.
    pub fn factor(&self, label: &LabelRef) -> f64 {
        self.0.labels.get(label).copied().unwrap_or(self.0.scale)
    }
}

/// `factor`, checked to be a scale: finite and non-negative.
fn checked(of: &str, factor: f64) -> Result<f64> {
    if factor.is_finite() && factor >= 0.0 {
        Ok(factor)
    } else {
        Err(Error::configuration(format!(
            "calibration multiplier for {of} must be finite and non-negative, got {factor}"
        )))
    }
}

impl TryFrom<Multipliers> for Calibration {
    type Error = Error;

    fn try_from(multipliers: Multipliers) -> Result<Self> {
        multipliers.labels.into_iter().try_fold(
            Self::scale(multipliers.scale)?,
            |calibration, (label, factor)| calibration.with_label(label, factor),
        )
    }
}

impl From<Calibration> for Multipliers {
    fn from(calibration: Calibration) -> Self {
        calibration.0
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    #[test]
    fn a_label_takes_its_own_multiplier_over_the_scale() {
        let calibration = Calibration::scale(0.8)
            .unwrap()
            .with_label("ORGANIZATION", 0.5)
            .unwrap();
        assert_eq!(calibration.factor(&LabelRef::new("PERSON_NAME")), 0.8);
        assert_eq!(calibration.factor(&LabelRef::new("ORGANIZATION")), 0.5);
    }

    #[test]
    fn a_negative_or_non_finite_multiplier_is_refused() {
        for bad in [-0.5, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Calibration::scale(bad).unwrap_err().kind(),
                ErrorKind::Configuration
            );
            let labelled = Calibration::scale(1.0).unwrap().with_label("PERSON", bad);
            assert_eq!(labelled.unwrap_err().kind(), ErrorKind::Configuration);
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserializing_checks_every_multiplier() {
        let calibration: Calibration =
            serde_json::from_str(r#"{"scale": 0.8, "labels": {"PERSON": 0.5}}"#).unwrap();
        assert_eq!(calibration.factor(&LabelRef::new("PERSON")), 0.5);
        assert_eq!(calibration.factor(&LabelRef::new("EMAIL")), 0.8);

        let labels_only: Calibration =
            serde_json::from_str(r#"{"labels": {"PERSON": 0.5}}"#).unwrap();
        assert_eq!(labels_only.factor(&LabelRef::new("EMAIL")), 1.0);

        for bad in [r#"{"scale": -1.0}"#, r#"{"labels": {"PERSON": -1.0}}"#] {
            assert!(serde_json::from_str::<Calibration>(bad).is_err());
        }
    }
}
