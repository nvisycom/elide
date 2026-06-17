//! The [`LabelCatalog`] registry.

use std::collections::HashMap;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Label, LabelRef};

/// A registry of [`Label`]s, keyed by name.
///
/// Holds the authoritative definitions (names + descriptions) for a run.
/// A [`LabelRef`] carried on a detection or entity is resolved back to
/// its full [`Label`] with [`get`].
///
/// [`get`]: LabelCatalog::get
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct LabelCatalog(HashMap<HipStr<'static>, Label>);

impl LabelCatalog {
    /// An empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// A catalog pre-populated with every built-in label.
    ///
    /// Walks [`builtins::BUILT_INS`] and registers each constant by name.
    /// Register custom labels alongside the built-ins with [`insert`].
    ///
    /// [`builtins::BUILT_INS`]: super::builtins
    /// [`insert`]: LabelCatalog::insert
    pub fn with_builtins() -> Self {
        super::builtins::BUILT_INS
            .iter()
            .map(|label| (**label).clone())
            .collect()
    }

    /// Insert a label, returning the previous definition for its name, if
    /// any.
    pub fn insert(&mut self, label: Label) -> Option<Label> {
        self.0.insert(label.name_owned(), label)
    }

    /// Resolve a reference to its full label definition.
    pub fn get(&self, label: &LabelRef) -> Option<&Label> {
        self.0.get(label.as_str())
    }

    /// Whether the catalog defines a label for `label`.
    pub fn contains(&self, label: &LabelRef) -> bool {
        self.0.contains_key(label.as_str())
    }

    /// The number of labels in the catalog.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<Label> for LabelCatalog {
    fn from_iter<I: IntoIterator<Item = Label>>(labels: I) -> Self {
        Self(labels.into_iter().map(|l| (l.name_owned(), l)).collect())
    }
}
