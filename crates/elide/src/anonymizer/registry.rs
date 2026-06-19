//! The [`OperatorRegistry`]: an ordered list of `(matcher, operator)`
//! rules resolving which operator hides which entity.
//!
//! Rules are tried in registration order; the first whose matcher accepts
//! the entity wins. An exact-label mapping, a tag mapping, an arbitrary
//! predicate, and a catch-all fallback are all just matchers, so one
//! ordered list expresses every selection policy with no hidden
//! precedence between kinds.

use std::sync::Arc;

use elide_core::entity::{Entity, LabelCatalog, LabelRef};
use elide_core::modality::Modality;
use elide_core::redaction::Operator;

use super::dyn_operator::DynOperator;

/// Boxed predicate over an entity, used by [`Matcher::Predicate`].
pub(crate) type Predicate<M> = Box<dyn Fn(&Entity<M>) -> bool + Send + Sync>;

/// How a rule decides whether it applies to an entity.
pub(crate) enum Matcher<M: Modality> {
    /// Exact label-name match.
    Label(LabelRef),
    /// The entity's label carries this tag (resolved through the
    /// [`LabelCatalog`]). Without a catalog, a tag matcher never matches.
    Tag(String),
    /// An arbitrary predicate over the entity.
    Predicate(Predicate<M>),
    /// Matches every entity. The catch-all fallback.
    Always,
}

impl<M: Modality> Matcher<M> {
    /// Whether this matcher accepts `entity`, given the catalog used to
    /// resolve tags.
    fn matches(&self, entity: &Entity<M>, catalog: Option<&LabelCatalog>) -> bool {
        match self {
            Matcher::Label(label) => &entity.label == label,
            Matcher::Tag(tag) => catalog
                .and_then(|c| c.get(&entity.label))
                .is_some_and(|label| label.has_tag(tag)),
            Matcher::Predicate(predicate) => predicate(entity),
            Matcher::Always => true,
        }
    }
}

/// One selection rule: a matcher and the operator to run when it accepts.
struct Rule<M: Modality> {
    matcher: Matcher<M>,
    operator: Arc<dyn DynOperator<M>>,
}

/// Ordered list of selection rules plus the catalog tag matchers consult.
///
/// Resolving an entity walks the rules in order and returns the first
/// matching operator. An entity that matches no rule is left untouched.
pub(crate) struct OperatorRegistry<M: Modality> {
    rules: Vec<Rule<M>>,
    catalog: Option<LabelCatalog>,
}

impl<M: Modality> OperatorRegistry<M> {
    /// An empty registry.
    pub(crate) fn new() -> Self {
        Self {
            rules: Vec::new(),
            catalog: None,
        }
    }

    /// Set the catalog tag matchers resolve label names against.
    pub(crate) fn set_catalog(&mut self, catalog: LabelCatalog) {
        self.catalog = Some(catalog);
    }

    /// Append a rule pairing `matcher` with `operator`.
    pub(crate) fn push<O: Operator<M> + 'static>(&mut self, matcher: Matcher<M>, operator: O) {
        self.rules.push(Rule {
            matcher,
            operator: Arc::new(operator),
        });
    }

    /// Resolve the operator for `entity`: the first rule whose matcher
    /// accepts it, or `None` when no rule matches.
    pub(crate) fn resolve(&self, entity: &Entity<M>) -> Option<&Arc<dyn DynOperator<M>>> {
        self.rules
            .iter()
            .find(|rule| rule.matcher.matches(entity, self.catalog.as_ref()))
            .map(|rule| &rule.operator)
    }
}

impl<M: Modality> Default for OperatorRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}
