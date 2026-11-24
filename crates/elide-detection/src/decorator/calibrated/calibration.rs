//! [`Calibration`]: how a [`Calibrated`](super::Calibrated)
//! rescales its recognizer's confidence, by [`Multiplier`]s.

use std::collections::HashMap;

use elide_core::entity::LabelRef;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Multiplier;

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
/// Every multiplier is a [`Multiplier`], checked as it is made, so building a
/// calibration cannot fail. The default leaves every label as it is.
///
/// ```
/// use elide_core::entity::builtins;
/// use elide_detection::decorator::calibrated::{Calibration, Multiplier};
///
/// const REGEX: Multiplier = Multiplier::new(0.8).unwrap();
/// const ORGANIZATIONS: Multiplier = Multiplier::new(0.6).unwrap();
///
/// let calibration = Calibration::scale(REGEX)
///     .with_label(builtins::ORGANIZATION_NAME.to_ref(), ORGANIZATIONS);
/// assert_eq!(calibration.factor(&builtins::PERSON_NAME.to_ref()), REGEX);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Calibration {
    /// The multiplier for a label with none of its own.
    #[cfg_attr(feature = "serde", serde(default))]
    scale: Multiplier,
    /// The multipliers for labels whose scores run differently.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    labels: HashMap<LabelRef, Multiplier>,
}

impl Calibration {
    /// Scale every label's confidence by `factor`.
    pub fn scale(factor: Multiplier) -> Self {
        Self {
            scale: factor,
            labels: HashMap::new(),
        }
    }

    /// Scale `label`'s confidence by `factor` instead.
    #[must_use]
    pub fn with_label(mut self, label: LabelRef, factor: Multiplier) -> Self {
        self.labels.insert(label, factor);
        self
    }

    /// The multiplier for `label`: its own, or the one for every label.
    pub fn factor(&self, label: &LabelRef) -> Multiplier {
        self.labels.get(label).copied().unwrap_or(self.scale)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::builtins;

    use super::*;

    fn multiplier(factor: f64) -> Multiplier {
        Multiplier::new(factor).unwrap()
    }

    #[test]
    fn a_label_takes_its_own_multiplier_over_the_scale() {
        let organization = builtins::ORGANIZATION_NAME.to_ref();
        let calibration =
            Calibration::scale(multiplier(0.8)).with_label(organization.clone(), multiplier(0.5));
        assert_eq!(
            calibration.factor(&builtins::PERSON_NAME.to_ref()),
            multiplier(0.8)
        );
        assert_eq!(calibration.factor(&organization), multiplier(0.5));
    }

    #[test]
    fn the_default_leaves_every_label_as_it_is() {
        let calibration = Calibration::default();
        assert_eq!(
            calibration.factor(&builtins::PERSON_NAME.to_ref()),
            Multiplier::IDENTITY
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserializing_checks_every_multiplier() {
        let calibration: Calibration =
            serde_json::from_str(r#"{"scale": 0.8, "labels": {"person_name": 0.5}}"#).unwrap();
        assert_eq!(
            calibration.factor(&builtins::PERSON_NAME.to_ref()),
            multiplier(0.5)
        );
        assert_eq!(
            calibration.factor(&builtins::EMAIL_ADDRESS.to_ref()),
            multiplier(0.8)
        );

        let labels_only: Calibration =
            serde_json::from_str(r#"{"labels": {"person_name": 0.5}}"#).unwrap();
        assert_eq!(
            labels_only.factor(&builtins::EMAIL_ADDRESS.to_ref()),
            Multiplier::IDENTITY
        );

        for bad in [r#"{"scale": -1.0}"#, r#"{"labels": {"person_name": -1.0}}"#] {
            assert!(serde_json::from_str::<Calibration>(bad).is_err());
        }
    }
}
