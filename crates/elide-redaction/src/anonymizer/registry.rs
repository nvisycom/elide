//! The [`OperatorRegistry`]: an ordered list of [`Rule`]s resolving which
//! operator hides which entity.
//!
//! Rules are tried in registration order; the first whose matcher accepts
//! the entity wins. An exact-label mapping, a tag mapping, an arbitrary
//! predicate, and a catch-all fallback are all just matchers, so one
//! ordered list expresses every selection policy with no hidden
//! precedence between kinds.

use elide_core::entity::provenance::{Attribution, RuleMatch};
use elide_core::entity::{Entity, LabelCatalog};
use elide_core::modality::Modality;
use elide_core::recognition::Scope;

use super::{Rule, SharedOperator};

/// What [`OperatorRegistry::resolve`] produces for a matched entity.
pub(crate) struct Resolved<'a, M: Modality> {
    /// The operator the matched rule binds.
    pub(crate) operator: &'a SharedOperator<M>,
    /// A summary of *which* rule matched (the automatic "why").
    pub(crate) matched_by: RuleMatch,
    /// The matched rule's author-supplied attribution (the policy "why").
    pub(crate) attribution: Option<&'a Attribution>,
}

/// Ordered list of selection rules plus the catalog tag matchers consult.
///
/// Resolving an entity walks the rules in order and returns the first
/// matching operator. An entity that matches no rule is left untouched.
pub(crate) struct OperatorRegistry<M: Modality> {
    rules: Vec<Rule<M>>,
    catalog: LabelCatalog,
}

impl<M: Modality> OperatorRegistry<M> {
    /// An empty registry, with an empty catalog.
    pub(crate) fn new() -> Self {
        Self {
            rules: Vec::new(),
            catalog: LabelCatalog::new(),
        }
    }

    /// Set the catalog tag matchers resolve label names against, and that
    /// predicates receive.
    pub(crate) fn set_catalog(&mut self, catalog: LabelCatalog) {
        self.catalog = catalog;
    }

    /// Append a [`Rule`] to the ordered list.
    pub(crate) fn push(&mut self, rule: Rule<M>) {
        self.rules.push(rule);
    }

    /// Append every [`Rule`] from `rules` to the ordered list, in order.
    pub(crate) fn extend(&mut self, rules: impl IntoIterator<Item = Rule<M>>) {
        self.rules.extend(rules);
    }

    /// Resolve the operator for `entity`: the first rule whose matcher
    /// accepts it, with a [`RuleMatch`] summary of *why* it matched and the
    /// rule's [`Attribution`] (the policy "why"), or `None` when no rule
    /// matches. `scope` is passed to scope-aware predicate rules.
    pub(crate) fn resolve(&self, entity: &Entity<M>, scope: &Scope) -> Option<Resolved<'_, M>> {
        self.rules
            .iter()
            .find(|rule| rule.matches(entity, &self.catalog, scope))
            .map(|rule| Resolved {
                operator: rule.operator(),
                matched_by: rule.to_rule_match(),
                attribution: rule.attribution(),
            })
    }
}

impl<M: Modality> Default for OperatorRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}
