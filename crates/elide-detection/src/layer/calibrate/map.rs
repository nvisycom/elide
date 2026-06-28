//! The [`CalibrationMap`] of per-recognizer multipliers.

use std::collections::HashMap;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Per-recognizer confidence multipliers applied before fusion.
///
/// Maps a recognizer name to a scaling factor, compensating for
/// score-distribution differences between detectors — a regex that
/// always returns `1.0` and an NER model that returns `0.3–0.9` can be
/// brought into the same range (e.g. a `0.8` multiplier on the regex)
/// before deduplication compares them. Recognizers absent from the map
/// are left unchanged (implicit multiplier `1.0`).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct CalibrationMap(HashMap<String, f64>);

impl CalibrationMap {
    /// An empty calibration map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the multiplier for a recognizer name.
    pub fn insert(&mut self, recognizer: impl Into<String>, multiplier: f64) {
        self.0.insert(recognizer.into(), multiplier);
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

impl<K, V> FromIterator<(K, V)> for CalibrationMap
where
    K: Into<String>,
    V: Into<f64>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}
