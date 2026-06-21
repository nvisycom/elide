//! [`LabelMap`] raw-to-canonical label translation table.

use std::collections::HashMap;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::{LabelCatalog, LabelRef};

/// Translation table from a model's raw label strings to the toolkit's
/// canonical entity labels.
///
/// A model may emit labels in its own vocabulary (`"PER"`, `"LOC"`,
/// `"B-ORG"`) rather than the canonical taxonomy the rest of the toolkit
/// speaks (`"person_name"`, `"location"`, `"organization"`). A `LabelMap`
/// maps each raw string to its canonical [`LabelRef`].
///
/// It is a utility for the *boundary* that adapts such a model — a NER
/// backend that wraps a fixed-label or BIO-tagged model applies it before
/// returning its spans, so the spans it emits already carry canonical
/// labels and downstream code never sees the raw vocabulary. Backends that
/// are given the canonical labels up front (zero-shot models) don't need
/// it. Lives here in `elide-core` so any modality's boundary can reuse it.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct LabelMap {
    entries: HashMap<HipStr<'static>, LabelRef>,
}

impl LabelMap {
    /// Empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Identity map over every label name in `catalog`: each name maps to
    /// a [`LabelRef`] of itself.
    ///
    /// Convenience for a boundary whose model already emits canonical label
    /// names (or has been calibrated to): [`get`] passes those labels
    /// through unchanged, and any name absent from `catalog` returns
    /// `None`.
    ///
    /// [`get`]: Self::get
    #[must_use]
    pub fn canonical(catalog: &LabelCatalog) -> Self {
        catalog
            .iter()
            .map(|label| (label.name().to_owned(), LabelRef::new(label.name())))
            .collect()
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

    /// Number of mappings.
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
