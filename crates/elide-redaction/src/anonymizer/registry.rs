//! The [`OperatorRegistry`]: an ordered list of `(matcher, operator)`
//! rules resolving which operator hides which entity.
//!
//! Rules are tried in registration order; the first whose matcher accepts
//! the entity wins. An exact-label mapping, a tag mapping, an arbitrary
//! predicate, and a catch-all fallback are all just matchers, so one
//! ordered list expresses every selection policy with no hidden
//! precedence between kinds.

use std::sync::Arc;

use elide_core::entity::provenance::{Attribution, RuleMatch};
use elide_core::entity::{Entity, LabelCatalog, LabelRef};
use elide_core::modality::Modality;
use elide_core::operator::Operator;
use hipstr::HipStr;

/// Boxed predicate over an entity, used by [`Matcher::Predicate`].
///
/// Receives the [`LabelCatalog`] (empty when none was set) so a predicate
/// can ask catalog-level questions — a label's tags or metadata — the same
/// way a [`Matcher::Tag`] resolves through it.
pub(crate) type Predicate<M> = Box<dyn Fn(&Entity<M>, &LabelCatalog) -> bool + Send + Sync>;

/// What [`OperatorRegistry::resolve`] produces for a matched entity.
pub(crate) struct Resolved<'a, M: Modality> {
    /// The operator the matched rule binds.
    pub(crate) operator: &'a Arc<dyn Operator<M>>,
    /// A summary of *which* rule matched (the automatic "why").
    pub(crate) matched_by: RuleMatch,
    /// The matched rule's author-supplied attribution (the policy "why").
    pub(crate) attribution: Option<&'a Attribution>,
}

/// How a rule decides whether it applies to an entity.
pub(crate) enum Matcher<M: Modality> {
    /// Exact label-name match.
    Label(LabelRef),
    /// The entity's label carries this tag (resolved through the
    /// [`LabelCatalog`]). An empty catalog never matches.
    Tag(HipStr<'static>),
    /// An arbitrary predicate over the entity.
    Predicate(Predicate<M>),
    /// Matches every entity. The catch-all fallback.
    Always,
}

impl<M: Modality> Matcher<M> {
    /// Whether this matcher accepts `entity`, given the catalog used to
    /// resolve tags (and passed through to predicates).
    fn matches(&self, entity: &Entity<M>, catalog: &LabelCatalog) -> bool {
        match self {
            Matcher::Label(label) => &entity.label == label,
            Matcher::Tag(tag) => catalog
                .get(&entity.label)
                .is_some_and(|label| label.has_tag(tag.as_str())),
            Matcher::Predicate(predicate) => predicate(entity, catalog),
            Matcher::Always => true,
        }
    }

    /// Summarise this matcher for provenance — the serializable "why" a
    /// rule fired, recorded on the entity's redaction event.
    fn to_rule_match(&self) -> RuleMatch {
        match self {
            Matcher::Label(label) => RuleMatch::Label(label.clone()),
            Matcher::Tag(tag) => RuleMatch::Tag(tag.clone()),
            Matcher::Predicate(_) => RuleMatch::Predicate,
            Matcher::Always => RuleMatch::Fallback,
        }
    }
}

/// One selection rule: a matcher, the operator to run when it accepts, and
/// an optional author-supplied [`Attribution`] (the policy "why").
struct Rule<M: Modality> {
    matcher: Matcher<M>,
    operator: Arc<dyn Operator<M>>,
    attribution: Option<Attribution>,
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

    /// Append a rule pairing `matcher` with `operator`, with no attribution.
    pub(crate) fn push<O: Operator<M> + 'static>(&mut self, matcher: Matcher<M>, operator: O) {
        self.rules.push(Rule {
            matcher,
            operator: Arc::new(operator),
            attribution: None,
        });
    }

    /// Attach `attribution` to the most-recently-pushed rule (the binding
    /// `.because` decorates). A no-op when no rule has been pushed yet.
    pub(crate) fn set_last_attribution(&mut self, attribution: Attribution) {
        if let Some(rule) = self.rules.last_mut() {
            rule.attribution = Some(attribution);
        }
    }

    /// Resolve the operator for `entity`: the first rule whose matcher
    /// accepts it, with a [`RuleMatch`] summary of *why* it matched and the
    /// rule's [`Attribution`] (the policy "why"), or `None` when no rule
    /// matches.
    pub(crate) fn resolve(&self, entity: &Entity<M>) -> Option<Resolved<'_, M>> {
        self.rules
            .iter()
            .find(|rule| rule.matcher.matches(entity, &self.catalog))
            .map(|rule| Resolved {
                operator: &rule.operator,
                matched_by: rule.matcher.to_rule_match(),
                attribution: rule.attribution.as_ref(),
            })
    }
}

impl<M: Modality> Default for OperatorRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}
