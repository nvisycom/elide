//! The [`ReversibleRegistry`]: an ordered list of `(matcher, operator)`
//! rules resolving which reversible operator recovers which entity.
//!
//! The reverse-direction counterpart to the anonymizer's registry, kept
//! deliberately small: reversal is keyed by label (you decrypt the labels
//! you encrypted) with a catch-all fallback. Rules are tried in
//! registration order; the first match wins.

use std::sync::Arc;

use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::Modality;
use elide_core::operator::ReversibleOperator;

use super::dyn_reversible::DynReversible;

/// How a rule decides whether it applies to an entity.
enum Matcher {
    /// Exact label-name match.
    Label(LabelRef),
    /// Matches every entity. The catch-all fallback.
    Always,
}

impl Matcher {
    fn matches<M: Modality>(&self, entity: &Entity<M>) -> bool {
        match self {
            Matcher::Label(label) => &entity.label == label,
            Matcher::Always => true,
        }
    }
}

/// One rule: a matcher and the reversible operator to run when it accepts.
struct Rule<M: Modality> {
    matcher: Matcher,
    operator: Arc<dyn DynReversible<M>>,
}

/// Ordered list of reversal rules.
///
/// Resolving an entity walks the rules in order and returns the first
/// matching operator. An entity that matches no rule is left untouched.
pub(crate) struct ReversibleRegistry<M: Modality> {
    rules: Vec<Rule<M>>,
}

impl<M: Modality> ReversibleRegistry<M> {
    /// An empty registry.
    pub(crate) fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Append a rule binding `operator` to an exact label.
    pub(crate) fn push_label<O: ReversibleOperator<M> + 'static>(
        &mut self,
        label: LabelRef,
        operator: O,
    ) {
        self.rules.push(Rule {
            matcher: Matcher::Label(label),
            operator: Arc::new(operator),
        });
    }

    /// Append a catch-all rule running `operator` for every unmatched entity.
    pub(crate) fn push_fallback<O: ReversibleOperator<M> + 'static>(&mut self, operator: O) {
        self.rules.push(Rule {
            matcher: Matcher::Always,
            operator: Arc::new(operator),
        });
    }

    /// Resolve the operator for `entity`: the first rule whose matcher
    /// accepts it, or `None` when no rule matches.
    pub(crate) fn resolve(&self, entity: &Entity<M>) -> Option<&Arc<dyn DynReversible<M>>> {
        self.rules
            .iter()
            .find(|rule| rule.matcher.matches(entity))
            .map(|rule| &rule.operator)
    }
}

impl<M: Modality> Default for ReversibleRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}
