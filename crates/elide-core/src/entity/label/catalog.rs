//! [`LabelCatalog`] registry.

use std::collections::HashMap;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::builtins::BUILT_INS;
use super::{Category, Label, LabelRef};
use crate::entity::Entity;
use crate::modality::Modality;

/// Registry of [`Label`]s, keyed by id.
///
/// Holds the authoritative definitions (localized names + descriptions)
/// for a run.
/// A [`LabelRef`] carried on a detection or entity is resolved back to
/// its full [`Label`] with [`get`].
///
/// [`get`]: LabelCatalog::get
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct LabelCatalog(
    #[cfg_attr(
        feature = "schema",
        schemars(with = "std::collections::HashMap<String, Label>")
    )]
    HashMap<HipStr<'static>, Label>,
);

impl LabelCatalog {
    /// Empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Catalog pre-populated with every built-in label.
    ///
    /// Walks [`builtins::BUILT_INS`] and registers each constant by id.
    /// Register custom labels alongside the built-ins with [`insert`].
    ///
    /// [`builtins::BUILT_INS`]: super::builtins
    /// [`insert`]: LabelCatalog::insert
    pub fn with_builtins() -> Self {
        BUILT_INS.iter().map(|label| (**label).clone()).collect()
    }

    /// Insert a label, returning the previous definition for its id, if
    /// any.
    pub fn insert(&mut self, label: Label) -> Option<Label> {
        self.0.insert(label.id_owned(), label)
    }

    /// Resolve a reference to its full label definition.
    pub fn get(&self, label: &LabelRef) -> Option<&Label> {
        self.0.get(label.as_str())
    }

    /// Whether the catalog defines a label for `label`.
    pub fn contains(&self, label: &LabelRef) -> bool {
        self.0.contains_key(label.as_str())
    }

    /// Keep only the entities whose label this catalog declares, dropping the
    /// rest.
    ///
    /// The output-restriction counterpart to [`contains`](Self::contains): a
    /// detection pipeline may emit entities the caller did not ask for (so a
    /// strong out-of-catalog match can still subsume a weak in-catalog one
    /// during reconciliation) and cull them here, after reconciliation, so only
    /// the requested types reach the caller. Called only with a non-empty
    /// catalog, an empty request detects nothing and is gated before
    /// recognition.
    pub fn retain_declared<M>(&self, entities: Vec<Entity<M>>) -> Vec<Entity<M>>
    where
        M: Modality,
    {
        entities
            .into_iter()
            .filter(|entity| self.contains(&entity.label))
            .collect()
    }

    /// The [`Category`] of `label`, resolved through this catalog.
    ///
    /// `None` when the catalog does not define `label`, or defines it without a
    /// category.
    pub fn category(&self, label: &LabelRef) -> Option<&Category> {
        self.get(label).and_then(Label::category)
    }

    /// Number of labels in the catalog.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over every [`Label`] in the catalog.
    pub fn iter(&self) -> impl Iterator<Item = &Label> {
        self.0.values()
    }

    /// A [`LabelRef`] for every label in the catalog, the set of labels
    /// recognizers are asked to emit (zero-shot NER, LLM prompt targets).
    pub fn refs(&self) -> impl Iterator<Item = LabelRef> + '_ {
        self.0.values().map(|label| label.to_ref())
    }

    /// Every label in the catalog carrying `tag`.
    ///
    /// A taxonomy filter, not a policy: built-in labels carry cross-cutting
    /// tags (`pii`, `phi`, `pci`, …), so `tagged("phi")` yields the health
    /// identifiers a caller's regulatory profile might target. The caller
    /// decides what to do with them.
    pub fn tagged<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Label> + 'a {
        self.0.values().filter(move |label| label.has_tag(tag))
    }
}

impl FromIterator<Label> for LabelCatalog {
    fn from_iter<I: IntoIterator<Item = Label>>(labels: I) -> Self {
        Self(labels.into_iter().map(|l| (l.id_owned(), l)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::builtins;
    use super::*;

    #[test]
    fn tag_filter_selects_matching_labels() {
        let catalog = LabelCatalog::with_builtins();

        // Every `tagged` label carries the tag, and only those.
        assert!(catalog.tagged("phi").all(|l| l.has_tag("phi")));
        let phi: Vec<&str> = catalog.tagged("phi").map(Label::id).collect();
        assert!(phi.contains(&builtins::MEDICAL_ID.id()));
        assert!(!phi.contains(&builtins::EMAIL_ADDRESS.id()));

        // An unknown tag yields nothing.
        assert_eq!(catalog.tagged("nonexistent").count(), 0);
    }

    #[test]
    fn retain_declared_keeps_only_in_catalog_labels() {
        use crate::entity::audit::{AuditEvent, PatternRule};
        use crate::modality::text::{Text, TextLocation};
        use crate::primitive::Confidence;

        fn entity(label: &str) -> Entity<Text> {
            let loc = TextLocation::inline(0..1);
            let conf = Confidence::new(0.9).unwrap();
            let event = AuditEvent::pattern("t", conf, loc.clone(), PatternRule::default());
            Entity::new(LabelRef::new(label), loc).with_event(event)
        }

        let catalog: LabelCatalog = [Label::new("email_address", "email")].into_iter().collect();
        let kept = catalog.retain_declared(vec![entity("email_address"), entity("iban")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].label, LabelRef::new("email_address"));

        // An empty catalog declares no type, so it keeps nothing.
        let none =
            LabelCatalog::new().retain_declared(vec![entity("email_address"), entity("iban")]);
        assert!(none.is_empty());
    }

    #[test]
    fn category_resolves_a_ref_through_the_catalog() {
        let catalog = LabelCatalog::with_builtins();
        assert_eq!(
            catalog
                .category(&builtins::IBAN.to_ref())
                .map(Category::as_str),
            Some("financial"),
        );
        // Unknown ref -> no category.
        assert!(catalog.category(&LabelRef::new("nope")).is_none());
    }
}
