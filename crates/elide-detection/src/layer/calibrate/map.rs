//! The [`CalibrationMap`] of per-recognizer multipliers.

use std::collections::HashMap;

use elide_core::{Error, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Per-recognizer confidence multipliers applied before fusion.
///
/// Maps a recognizer name to a scaling factor, compensating for
/// score-distribution differences between detectors, a regex that
/// always returns `1.0` and an NER model that returns `0.3–0.9` can be
/// brought into the same range (e.g. a `0.8` multiplier on the regex)
/// before deduplication compares them. Recognizers absent from the map
/// are left unchanged (implicit multiplier `1.0`).
///
/// A multiplier is finite and non-negative, however the map is built (insert,
/// [`from_multipliers`](Self::from_multipliers), or deserialized): a negative
/// or NaN multiplier has no meaning as a scale and would zero a recognizer's
/// scores, and an infinite one turns a zero score into NaN. A large finite
/// multiplier is allowed: scaled confidence clamps to `[0, 1]`.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(try_from = "HashMap<String, f64>", into = "HashMap<String, f64>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CalibrationMap(HashMap<String, f64>);

impl CalibrationMap {
    /// An empty calibration map.
    pub fn new() -> Self {
        Self::default()
    }

    /// A map of the `(recognizer, multiplier)` pairs.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Configuration`](elide_core::ErrorKind::Configuration) if a
    /// multiplier is negative or not finite.
    pub fn from_multipliers<K, V>(multipliers: impl IntoIterator<Item = (K, V)>) -> Result<Self>
    where
        K: Into<String>,
        V: Into<f64>,
    {
        let mut map = Self::new();
        for (recognizer, multiplier) in multipliers {
            map.insert(recognizer, multiplier)?;
        }
        Ok(map)
    }

    /// Set the multiplier for a recognizer name.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Configuration`](elide_core::ErrorKind::Configuration) if
    /// `multiplier` is negative or not finite.
    pub fn insert(
        &mut self,
        recognizer: impl Into<String>,
        multiplier: impl Into<f64>,
    ) -> Result<()> {
        let recognizer = recognizer.into();
        let multiplier = multiplier.into();
        if !multiplier.is_finite() || multiplier < 0.0 {
            return Err(Error::configuration(format!(
                "calibration multiplier for `{recognizer}` must be finite and non-negative, got {multiplier}"
            )));
        }
        self.0.insert(recognizer, multiplier);
        Ok(())
    }

    /// The multiplier for a recognizer name, or `None`.
    pub fn get(&self, recognizer: &str) -> Option<f64> {
        self.0.get(recognizer).copied()
    }

    /// Whether no multipliers are registered.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The number of registered multipliers.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl TryFrom<HashMap<String, f64>> for CalibrationMap {
    type Error = Error;

    fn try_from(multipliers: HashMap<String, f64>) -> Result<Self> {
        Self::from_multipliers(multipliers)
    }
}

impl From<CalibrationMap> for HashMap<String, f64> {
    fn from(map: CalibrationMap) -> Self {
        map.0
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    #[test]
    fn a_finite_non_negative_multiplier_is_kept() {
        let map = CalibrationMap::from_multipliers([("regex", 0.8), ("ner", 0.0)]).unwrap();
        assert_eq!(map.get("regex"), Some(0.8));
        assert_eq!(map.get("ner"), Some(0.0));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserializing_checks_every_multiplier() {
        let map: CalibrationMap = serde_json::from_str(r#"{"regex": 0.8}"#).unwrap();
        assert_eq!(map.get("regex"), Some(0.8));
        assert!(serde_json::from_str::<CalibrationMap>(r#"{"regex": -1.0}"#).is_err());
    }

    #[cfg(feature = "schema")]
    #[test]
    fn the_schema_is_a_map_of_multipliers() {
        let schema = serde_json::to_value(schemars::schema_for!(CalibrationMap)).unwrap();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"]["type"], "number");
    }

    #[test]
    fn a_negative_or_non_finite_multiplier_is_refused() {
        for bad in [-0.5, f64::NAN, f64::INFINITY] {
            let err = CalibrationMap::new().insert("regex", bad).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Configuration);
        }
    }
}
