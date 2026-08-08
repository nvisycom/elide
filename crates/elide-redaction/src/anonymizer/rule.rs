//! [`Rule`]: a first-class anonymizer selection rule — a matcher, an
//! operator, and an optional policy [`Attribution`].

use std::sync::Arc;

use elide_core::entity::provenance::{Attribution, RuleMatch};
use elide_core::entity::{Entity, LabelCatalog, LabelRef};
use elide_core::modality::Modality;
use elide_core::operator::Operator;
use elide_core::recognition::Scope;
use hipstr::HipStr;

use super::{Predicate, SharedOperator};

/// The context a [predicate rule](Rule::predicate) matches against.
///
/// A read-only bundle of everything a matcher can branch on: the
/// [`entity`](Self::entity) under test, the [`catalog`](Self::catalog) that
/// resolves a label's tags and metadata (empty when none was set), and the
/// run [`scope`](Self::scope) carrying request context (purpose, audience,
/// tags, languages, countries). A predicate reads whichever fields it needs
/// and ignores the rest; new context lands here without changing any
/// predicate's signature.
///
/// [`Scope`]: elide_core::recognition::Scope
#[non_exhaustive]
pub struct MatchContext<'a, M: Modality> {
    /// The entity being matched — its label, confidence, location, and
    /// provenance.
    pub entity: &'a Entity<M>,
    /// The label catalog, for tag- and metadata-aware matching. Empty when
    /// no catalog was set on the anonymizer.
    pub catalog: &'a LabelCatalog,
    /// The caller-asserted request [`Scope`], for scope-aware matching.
    ///
    /// [`Scope`]: elide_core::recognition::Scope
    pub scope: &'a Scope,
}

/// How a [`Rule`] decides whether it applies to an entity.
///
/// An exact label, a label tag (resolved through the [`LabelCatalog`]), an
/// arbitrary predicate, or a catch-all — one closed set so the ordered rule
/// list has no hidden precedence between kinds.
pub(crate) enum Matcher<M: Modality> {
    /// Exact label-name match.
    Label(LabelRef),
    /// The entity's label carries this tag (resolved through the catalog).
    /// An empty catalog never matches.
    Tag(HipStr<'static>),
    /// An arbitrary predicate over the entity (with the catalog and scope).
    Predicate(Predicate<M>),
    /// Matches every entity — the catch-all fallback.
    Always,
}

impl<M: Modality> Matcher<M> {
    /// Whether this matcher accepts `entity`, given the catalog used to
    /// resolve tags and the run [`Scope`] (both passed through to predicates).
    ///
    /// [`Scope`]: elide_core::recognition::Scope
    fn matches(&self, entity: &Entity<M>, catalog: &LabelCatalog, scope: &Scope) -> bool {
        match self {
            Matcher::Label(label) => &entity.label == label,
            Matcher::Tag(tag) => catalog
                .get(&entity.label)
                .is_some_and(|label| label.has_tag(tag.as_str())),
            Matcher::Predicate(predicate) => predicate(&MatchContext {
                entity,
                catalog,
                scope,
            }),
            Matcher::Always => true,
        }
    }

    /// Summarise this matcher for provenance — the serializable "why" a rule
    /// fired, recorded on the entity's redaction event.
    fn to_rule_match(&self) -> RuleMatch {
        match self {
            Matcher::Label(label) => RuleMatch::Label(label.clone()),
            Matcher::Tag(tag) => RuleMatch::Tag(tag.clone()),
            Matcher::Predicate(_) => RuleMatch::Predicate,
            Matcher::Always => RuleMatch::Fallback,
        }
    }
}

/// A matcher, the operator it runs, and an optional policy [`Attribution`].
///
/// A [`Rule`] is a self-contained, first-class value: build it, attribute
/// it with [`because`](Self::because), and hand it to [`Anonymizer::with`]
/// / [`Anonymizer::with_multiple`]. Because the attribution is a field on the
/// rule, it is bound to *this* rule structurally — there is no positional
/// "attribute the last rule" step.
///
/// Rules are tried in the order they are added; the first whose matcher
/// accepts an entity wins.
///
/// ```ignore
/// Rule::label(EMAIL_ADDRESS, Replace::default()).because("gdpr-art-17")
/// Rule::tag("financial", Mask::stars())
/// Rule::fallback(Erase)
/// ```
///
/// [`Anonymizer::with`]: super::Anonymizer::with
/// [`Anonymizer::with_multiple`]: super::Anonymizer::with_multiple
#[must_use]
pub struct Rule<M: Modality> {
    matcher: Matcher<M>,
    operator: SharedOperator<M>,
    attribution: Option<Attribution>,
}

impl<M: Modality> Rule<M> {
    /// Build a rule from a matcher and operator, with no attribution.
    fn new<O: Operator<M> + 'static>(matcher: Matcher<M>, operator: O) -> Self {
        Self {
            matcher,
            operator: Arc::new(operator),
            attribution: None,
        }
    }

    /// A rule binding `operator` to an exact `label`.
    pub fn label<O: Operator<M> + 'static>(label: LabelRef, operator: O) -> Self {
        Self::new(Matcher::Label(label), operator)
    }

    /// A rule binding `operator` to every entity whose label carries `tag`.
    /// Requires a catalog on the anonymizer ([`Anonymizer::with_catalog`]).
    ///
    /// [`Anonymizer::with_catalog`]: super::Anonymizer::with_catalog
    pub fn tag<O: Operator<M> + 'static>(tag: impl Into<HipStr<'static>>, operator: O) -> Self {
        Self::new(Matcher::Tag(tag.into()), operator)
    }

    /// A rule binding `operator` to every entity `predicate` accepts.
    ///
    /// The predicate receives a [`MatchContext`] — the entity under test plus the
    /// [`LabelCatalog`] and run [`Scope`] — and reads whichever fields it
    /// needs. A confidence gate touches only `cx.entity`; a scope-aware rule
    /// reads `cx.scope`; a tag-aware one reads `cx.catalog`.
    ///
    /// Scope-aware matching is how one detected document is redacted
    /// differently per request context: a predicate branches on
    /// `cx.scope.metadata.audience` (or `purpose`, `tags`, `languages`,
    /// `countries`) and the caller runs selection once per [`Scope`], so the
    /// same analyzed entities produce a different plan for each audience.
    ///
    /// ```ignore
    /// // Auditors see more of a card than support agents do.
    /// Rule::predicate(
    ///     |cx| cx.scope.metadata.audience.iter().any(|a| a == "auditor"),
    ///     Mask::stars().with_keep_prefix(6).with_keep_suffix(4),
    /// )
    /// ```
    ///
    /// [`Scope`]: elide_core::recognition::Scope
    pub fn predicate<O, P>(predicate: P, operator: O) -> Self
    where
        O: Operator<M> + 'static,
        P: Fn(&MatchContext<'_, M>) -> bool + Send + Sync + 'static,
    {
        Self::new(Matcher::Predicate(Box::new(predicate)), operator)
    }

    /// A catch-all rule: `operator` runs for every entity not matched by an
    /// earlier rule. Any rule added after a fallback is unreachable.
    pub fn fallback<O: Operator<M> + 'static>(operator: O) -> Self {
        Self::new(Matcher::Always, operator)
    }

    /// Attribute this rule to a policy: the [`Attribution`] (a bare policy
    /// id, or one built with a reason) is recorded on the redaction
    /// provenance of every entity this rule redacts — the *why* alongside
    /// the matched rule.
    ///
    /// [`Attribution`]: elide_core::entity::provenance::Attribution
    pub fn because(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    /// This rule's operator (for the registry).
    pub(crate) fn operator(&self) -> &SharedOperator<M> {
        &self.operator
    }

    /// This rule's attribution, if any (for the registry).
    pub(crate) fn attribution(&self) -> Option<&Attribution> {
        self.attribution.as_ref()
    }

    /// Whether this rule's matcher accepts `entity` (for the registry).
    pub(crate) fn matches(
        &self,
        entity: &Entity<M>,
        catalog: &LabelCatalog,
        scope: &Scope,
    ) -> bool {
        self.matcher.matches(entity, catalog, scope)
    }

    /// The provenance summary of this rule's matcher (for the registry).
    pub(crate) fn to_rule_match(&self) -> RuleMatch {
        self.matcher.to_rule_match()
    }
}
