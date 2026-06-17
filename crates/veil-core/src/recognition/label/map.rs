//! The [`LabelMap`] raw-to-canonical label translation table.

use std::collections::HashMap;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::LabelRef;

/// A translation table from a backend's raw label strings to the
/// toolkit's canonical entity labels.
///
/// Recognizers — NER models especially — emit labels in their own
/// vocabulary (`"PER"`, `"LOC"`, `"B-ORG"`). A `LabelMap` maps each such
/// raw string to the [`LabelRef`] the rest of the model speaks
/// (`"PERSON"`, `"LOCATION"`, `"ORGANIZATION"`), so a recognizer can
/// translate its output at the boundary without the canonical taxonomy
/// leaking into the backend or vice versa.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct LabelMap {
    entries: HashMap<HipStr<'static>, LabelRef>,
}

impl LabelMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a mapping from a raw label to a canonical [`LabelRef`], returning
    /// the previous target for that raw label, if any.
    pub fn insert(&mut self, raw: impl Into<HipStr<'static>>, label: LabelRef) -> Option<LabelRef> {
        self.entries.insert(raw.into(), label)
    }

    /// Translate a raw backend label to its canonical [`LabelRef`].
    pub fn get(&self, raw: &str) -> Option<&LabelRef> {
        self.entries.get(raw)
    }

    /// Whether the map has a translation for `raw`.
    pub fn contains(&self, raw: &str) -> bool {
        self.entries.contains_key(raw)
    }

    /// The number of mappings.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<R> FromIterator<(R, LabelRef)> for LabelMap
where
    R: Into<HipStr<'static>>,
{
    fn from_iter<I: IntoIterator<Item = (R, LabelRef)>>(mappings: I) -> Self {
        Self {
            entries: mappings
                .into_iter()
                .map(|(raw, label)| (raw.into(), label))
                .collect(),
        }
    }
}
