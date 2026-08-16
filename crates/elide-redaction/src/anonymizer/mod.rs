//! The [`Anonymizer`] — the "hide" engine.
//!
//! The redaction counterpart to [`Analyzer`]: an ordered list of
//! selection rules plus its entry points. [`select`] resolves each
//! entity's operator into a reviewable [`Selection`] without touching the
//! data; [`anonymize`] selects, computes each [`Replacement`], and applies
//! the batch back into the target in one step; [`anonymize_selections`]
//! applies a (possibly reviewed) set of selections, each carrying the live
//! operator to run.
//!
//! [`Analyzer`]: crate::Analyzer
//! [`select`]: Anonymizer::select
//! [`anonymize`]: Anonymizer::anonymize
//! [`anonymize_selections`]: Anonymizer::anonymize_selections
//! [`Replacement`]: elide_core::modality::Modality::Replacement

mod registry;
mod rule;
mod selection;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::audit::AuditEvent;
use elide_core::entity::{Entity, LabelCatalog};
use elide_core::modality::{DataReader, DataWriter, Modality, ModalityLocation};
use elide_core::operator::{Operator, Redactions};
use elide_core::recognition::Scope;

use self::registry::OperatorRegistry;
pub use self::rule::{MatchContext, Rule};
pub use self::selection::{Selection, SelectionView};

/// An operator stored in a [`Rule`], type-erased and shared.
pub(crate) type SharedOperator<M> = Arc<dyn Operator<M>>;

/// Boxed predicate over a [`MatchContext`], used by `Matcher::Predicate`.
///
/// The context bundles the entity under test, the [`LabelCatalog`] (empty when
/// none was set), and the run [`Scope`], so a predicate can branch on the
/// entity, a label's tags or metadata, or request context (purpose, audience,
/// tags, languages, countries) — reading whichever fields it needs.
///
/// [`Scope`]: elide_core::recognition::Scope
pub(crate) type Predicate<M> = Box<dyn Fn(&MatchContext<'_, M>) -> bool + Send + Sync>;

/// The hide engine: selects an operator per entity and computes its
/// replacement.
///
/// Generic over the [`Modality`] `M`. Selection is an *ordered list of*
/// [`Rule`]s, tried top to bottom with the first match winning. Add each
/// rule with [`with`](Self::with) (or several with [`with_multiple`]); a rule
/// binds an operator to an exact label, a label tag (which needs a catalog,
/// see [`with_catalog`]), an arbitrary predicate, or a catch-all fallback,
/// and optionally carries a policy [`Attribution`]. [`anonymize`] resolves
/// and runs the operators, applying the replacements back into the target.
///
/// ```ignore
/// Anonymizer::new()
///     .with_catalog(LabelCatalog::with_builtins())
///     // Order matters: a weak detection is kept as-is before any
///     // label or tag rule can fire.
///     .with(Rule::predicate(|cx| !ConfidenceThreshold::BASELINE.passes(cx.entity.confidence), Keep))
///     .with(Rule::label(LabelRef::new("EMAIL_ADDRESS"), Replace::default()))
///     .with(Rule::tag("financial", Mask::stars()))
///     .with(Rule::fallback(Erase))
///     .anonymize(&mut document, &entities)
///     .await?;
/// ```
///
/// [`with_multiple`]: Anonymizer::with_multiple
/// [`with_catalog`]: Anonymizer::with_catalog
/// [`anonymize`]: Anonymizer::anonymize
/// [`Attribution`]: elide_core::entity::audit::Attribution
pub struct Anonymizer<M: Modality> {
    operators: OperatorRegistry<M>,
}

impl<M: Modality> Anonymizer<M> {
    /// An anonymizer with no rules.
    pub fn new() -> Self {
        Self {
            operators: OperatorRegistry::new(),
        }
    }

    /// Set the [`LabelCatalog`] that [tag rules](Rule::tag) resolve label
    /// names against. Without it, tag rules never match.
    #[must_use]
    pub fn with_catalog(mut self, catalog: LabelCatalog) -> Self {
        self.operators.set_catalog(catalog);
        self
    }

    /// Append a selection [`Rule`] to the ordered list.
    ///
    /// Rules are tried top to bottom, first match wins. Build a rule with
    /// one of the [`Rule`] constructors and optionally attribute it with
    /// [`Rule::because`]:
    ///
    /// ```ignore
    /// Anonymizer::new()
    ///     .with(Rule::label(EMAIL, Replace::default()).because("gdpr-art-17"))
    ///     .with(Rule::tag("financial", Mask::stars()))
    ///     .with(Rule::fallback(Erase));
    /// ```
    #[must_use]
    pub fn with(mut self, rule: Rule<M>) -> Self {
        self.operators.push(rule);
        self
    }

    /// Append several [`Rule`]s in order — the batch counterpart to
    /// [`with`](Self::with).
    ///
    /// Handy when a policy layer holds a `Vec<Rule<M>>`, or for a single
    /// logical rule that fans out across labels under one shared
    /// [`Attribution`]: build each entry with the same `.because(attr)` (a
    /// cheap clone) and add them together. Every entity a rule redacts
    /// records that rule's attribution, so the shared attribution is what a
    /// reviewer traces back to "which rule fired".
    ///
    /// ```ignore
    /// let attr = Attribution::freeform("hipaa-safe-harbor");
    /// Anonymizer::new().with_multiple([
    ///     Rule::label(DATE_OF_BIRTH, GeneralizeDate::new(Year)).because(attr.clone()),
    ///     Rule::label(AGE, Clamp::new().with_ceiling(90.0, "90 or older")).because(attr.clone()),
    ///     Rule::label(PERSON_NAME, Erase).because(attr),
    /// ]);
    /// ```
    ///
    /// [`Attribution`]: elide_core::entity::audit::Attribution
    #[must_use]
    pub fn with_multiple(mut self, rules: impl IntoIterator<Item = Rule<M>>) -> Self {
        self.operators.extend(rules);
        self
    }

    /// Resolve the reviewable operator [`Selection`] for every entity,
    /// without reading any data.
    ///
    /// This is the *decision* half of redaction, split out so it can be
    /// inspected — and later overridden — before anything is applied. For
    /// each overlap cluster it resolves the winning operator (see the merge
    /// rule below) and emits one `Selection` naming that operator, the
    /// entities it covers, the rule that matched, and any policy attribution.
    /// No document data is touched: `select` never reads the medium, so it is
    /// cheap to run and safe to run speculatively (e.g. to review picks for
    /// several audiences from one detection).
    ///
    /// `scope` is the caller-asserted request [`Scope`], passed to
    /// [predicate rules](Rule::predicate) via [`MatchContext::scope`] so selection
    /// can branch on request context — the seam that lets **one** analyzed set of
    /// entities be redacted differently per audience: call `select` once per
    /// [`Scope`], apply each result to a copy. Rules that aren't scope-aware
    /// ignore it.
    ///
    /// Entities whose label has no operator and no fallback are skipped — no
    /// `Selection` is emitted for them.
    ///
    /// [`Scope`]: elide_core::recognition::Scope
    ///
    /// **Overlapping entities are merged.** Where a set of entities overlap
    /// in the medium (a left-over nesting, or one a user re-introduced by
    /// editing the report), redacting each separately would write competing
    /// operators over the same bytes and corrupt the output. Instead the
    /// overlapping set collapses to *one* [`Selection`] covering the
    /// [union][union] of their spans, run by the **safest** operator among
    /// them — the one whose output leaks least (highest [`LeakProfile`]).
    /// Ties go to the wider span, then the earlier position. A purely
    /// mechanical safety step: it makes no semantic choice about which
    /// *finding* is right — that is detection's job.
    ///
    /// Feed the result to [`anonymize_selections`] to apply it, optionally
    /// after review. [`anonymize`] runs `select` and applies in one step.
    ///
    /// [`anonymize_selections`]: Self::anonymize_selections
    /// [`anonymize`]: Self::anonymize
    /// [union]: elide_core::modality::ModalityLocation::union
    /// [`LeakProfile`]: elide_core::operator::LeakProfile
    pub fn select(&self, entities: &[Entity<M>], scope: &Scope) -> Vec<Selection<M>> {
        let mut selections = Vec::new();
        for cluster in cluster_overlaps(entities) {
            // Pick the safest operator in the cluster — the one that leaks
            // least — to redact the whole overlapping span; ties go to the
            // wider span, then the earlier position. A singleton cluster just
            // resolves its one entity. `None` means no member had an operator.
            let Some((operator, matched_by, attribution)) = cluster
                .iter()
                .copied()
                .filter_map(|i| self.operators.resolve(&entities[i], scope).map(|r| (i, r)))
                .max_by(|(i, a), (j, b)| {
                    a.operator
                        .leak_profile()
                        .cmp(&b.operator.leak_profile())
                        .then_with(|| entities[*i].location.span_cmp(&entities[*j].location))
                        .then_with(|| entities[*j].location.position_cmp(&entities[*i].location))
                })
                .map(|(_, r)| (Arc::clone(r.operator), r.matched_by, r.attribution.cloned()))
            else {
                continue;
            };

            let covered = cluster.iter().map(|&i| entities[i].id).collect();
            selections.push(Selection::new(operator, covered, matched_by, attribution));
        }
        selections
    }

    /// Hide every entity by applying its operator's replacement back into
    /// `target`.
    ///
    /// The complete redaction step: [`select`]s each entity's operator,
    /// reads its value from `target`, runs the operator, and hands the batch
    /// to [`DataWriter::write_at`] so `target` owns the *how* and *ordering*
    /// of applying it. `target` is both the reader and the writer —
    /// typically a decoded codec document. Entities must already be in
    /// `target`'s coordinate system.
    ///
    /// `scope` is passed to [`select`] for scope-aware rules.
    ///
    /// Use [`select`] + [`anonymize_selections`] instead when the picks must
    /// be reviewed (or round-tripped) between selection and application.
    ///
    /// [`select`]: Self::select
    /// [`anonymize_selections`]: Self::anonymize_selections
    pub async fn anonymize<T>(
        &self,
        target: &mut T,
        entities: &mut [Entity<M>],
        scope: &Scope,
    ) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
    {
        let selections = self.select(entities, scope);
        let redactions = self.execute(entities, &selections, target).await?;
        target.write_at(redactions).await
    }

    /// Apply a set of (possibly reviewed) [`Selection`]s back into `target`.
    ///
    /// The apply half of the reviewable path: where [`select`] produced the
    /// picks and a reviewer may have edited them, this runs each selection's
    /// operator and writes the result. Each selection carries a live operator
    /// — either the one [`select`] resolved, or one a caller rebuilt from a
    /// [`SelectionView`] (resolving its [`operator_id`] through a registry) and
    /// passed to [`Selection::new`]. To override a pick, swap the selection's
    /// operator before calling this.
    ///
    /// Every entity a selection covers records a [`Redaction`] event, so
    /// provenance stays faithful for merged entities too.
    ///
    /// [`select`]: Self::select
    /// [`operator_id`]: SelectionView::operator_id
    /// [`Redaction`]: elide_core::entity::audit::AuditKind::Redaction
    pub async fn anonymize_selections<T>(
        &self,
        target: &mut T,
        entities: &mut [Entity<M>],
        selections: &[Selection<M>],
    ) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
    {
        let redactions = self.execute(entities, selections, target).await?;
        target.write_at(redactions).await
    }

    /// Read, run, and record each selection, returning the [`Redactions`]
    /// batch without writing it — the shared core of [`anonymize`] and
    /// [`anonymize_selections`].
    ///
    /// For each selection: union the spans of the entities it covers, read
    /// that span, run the selection's operator, and record a [`Redaction`]
    /// event on every covered entity. Selections whose span reads no data are
    /// skipped.
    ///
    /// [`anonymize`]: Self::anonymize
    /// [`anonymize_selections`]: Self::anonymize_selections
    /// [`Redaction`]: elide_core::entity::audit::AuditKind::Redaction
    async fn execute(
        &self,
        entities: &mut [Entity<M>],
        selections: &[Selection<M>],
        reader: &impl DataReader<M>,
    ) -> Result<Redactions<M>> {
        // Map entity ids back to their slice index so a selection (which
        // names its entities by id) can reach their locations and provenance.
        let index: std::collections::HashMap<_, _> = entities
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, i))
            .collect();

        let mut redactions = Redactions::new();
        for selection in selections {
            // The entity indices this selection covers, in slice order.
            let members: Vec<usize> = selection
                .entities()
                .iter()
                .filter_map(|id| index.get(id).copied())
                .collect();
            let Some(&winner) = members.first() else {
                continue;
            };
            let operator = selection.operator();

            // Redact the union of every member's span. Clustering groups only
            // entities that coalesce, so the fold never hits `None`; a
            // singleton unions to itself.
            let location = members
                .iter()
                .map(|&i| entities[i].location.clone())
                .reduce(|acc, loc| {
                    acc.union(&loc)
                        .expect("selection members coalesce by construction")
                })
                .expect("a selection covers at least one entity");
            let Some(data) = reader.read_at(&location).await? else {
                tracing::debug!(modality = M::NAME, "location read no data; skipping");
                continue;
            };
            let replacement = operator.anonymize(&entities[winner], &data).await?;

            // Record the redaction on every member, so each entity's
            // provenance reflects that this operator hid it.
            for &i in &members {
                let entity = &mut entities[i];
                let event = AuditEvent::redaction(
                    operator.id(),
                    operator.leak_profile(),
                    entity.confidence,
                    selection.matched_by().clone(),
                    selection.attribution().cloned(),
                );
                entity.audit.record(event);
            }
            redactions.push(location, replacement);
        }
        Ok(redactions)
    }
}

impl<M: Modality> Default for Anonymizer<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// Group entity indices that redact as one span, by single-linkage
/// clustering: two entities join when they overlap *and* their locations
/// [coalesce] into one span.
///
/// Each group is a `Vec` of indices into `entities`. Disjoint entities each
/// form a singleton; a chain of pairwise links (A–B, B–C) lands in one
/// group even if A and C don't touch, and an entity bridging two existing
/// groups merges them. Two entities that overlap but can't coalesce (the
/// same byte range on different pages, say) stay in separate groups, so
/// every group's [`union`][coalesce] is well-defined — no member is ever
/// dropped when the span is computed.
///
/// [coalesce]: ModalityLocation::union
fn cluster_overlaps<M: Modality>(entities: &[Entity<M>]) -> Vec<Vec<usize>> {
    // Two entities link only if they overlap and coalesce into one span, so
    // every group folds to a single union with no member lost.
    let links = |a: &M::Location, b: &M::Location| a.overlaps(b) && a.union(b).is_some();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in 0..entities.len() {
        let location = &entities[i].location;
        // Every existing group holding an entity this one links to. With more
        // than one, this entity bridges them, so they all merge.
        let hit: Vec<usize> = (0..groups.len())
            .filter(|&g| {
                groups[g]
                    .iter()
                    .any(|&other| links(&entities[other].location, location))
            })
            .collect();
        match hit.first().copied() {
            None => groups.push(vec![i]),
            Some(first) => {
                groups[first].push(i);
                // Remove the other bridged groups from the back so the lower
                // indices (including `first`) stay valid, folding each in.
                for &g in hit.iter().skip(1).rev() {
                    let merged = groups.remove(g);
                    groups[first].extend(merged);
                }
            }
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use elide_core::entity::LabelRef;
    use elide_core::entity::audit::{AuditEvent, AuditKind, AuditLog, PatternEvent, RuleMatch};
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::Confidence;
    use elide_operator::operators::{Erase, Mask, Replace};

    use super::*;

    /// In-memory text reader: slices the backing string by byte range. Also
    /// a no-op [`DataWriter`] so the apply-path entry points that write can be
    /// exercised without a full codec document.
    struct StrReader(String);

    #[async_trait::async_trait]
    impl DataReader<Text> for StrReader {
        async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
            Ok(self
                .0
                .get(location.range.start..location.range.end)
                .map(TextData::new))
        }
    }

    #[async_trait::async_trait]
    impl DataWriter<Text> for StrReader {
        async fn write_at(&mut self, _redactions: Redactions<Text>) -> Result<()> {
            Ok(())
        }
    }

    fn entity(label: &str, start: usize, end: usize) -> Entity<Text> {
        let loc = TextLocation::new(start, end);
        let confidence = Confidence::new(0.9).unwrap();
        let event = AuditEvent::pattern("t", confidence, loc.clone(), PatternEvent::default());
        Entity::new(LabelRef::new(label), loc, confidence, AuditLog::new(event))
    }

    /// `select` resolves one [`Selection`] per redaction, naming the winning
    /// operator and the entities it covers — without reading any data.
    #[tokio::test]
    async fn select_resolves_one_selection_per_redaction() {
        let entities = vec![entity("NAME", 0, 5), entity("URL", 10, 13)];
        let anonymizer = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Erase))
            .with(Rule::label(LabelRef::new("URL"), Replace::default()));

        let selections = anonymizer.select(&entities, &Scope::default());

        assert_eq!(selections.len(), 2, "one selection per disjoint entity");
        // Each selection covers exactly its own entity, by id.
        for (selection, entity) in selections.iter().zip(&entities) {
            assert_eq!(selection.entities(), [entity.id]);
        }
    }

    /// A [scope-aware predicate](Rule::predicate) reading [`MatchContext::scope`]
    /// lets the *same* entities be selected differently per request `Scope` —
    /// the multi-audience seam. Here an auditor keeps a wider prefix than a
    /// support agent.
    #[tokio::test]
    async fn scope_predicate_selects_per_audience() {
        let entities = vec![entity("PAYMENT_CARD", 0, 16)];
        let anonymizer = Anonymizer::new()
            // Auditors: keep the leading 6. Anyone else: mask everything.
            .with(Rule::predicate(
                |cx| cx.scope.metadata.audience.iter().any(|a| a == "auditor"),
                Mask::stars().with_keep_prefix(6),
            ))
            .with(Rule::fallback(Mask::stars()));

        let auditor = Scope::new().with_audience(["auditor"]);
        let support = Scope::new().with_audience(["support"]);

        // One detection, two scopes → two different rules fire. The predicate
        // rule matched the auditor (RuleMatch::Predicate); the support agent
        // fell through to the fallback (RuleMatch::Fallback).
        let for_auditor = anonymizer.select(&entities, &auditor);
        let for_support = anonymizer.select(&entities, &support);

        assert!(
            matches!(for_auditor[0].matched_by(), RuleMatch::Predicate),
            "auditor matches the scope predicate",
        );
        assert!(
            matches!(for_support[0].matched_by(), RuleMatch::Fallback),
            "support falls through to the fallback",
        );
    }

    /// Overlapping entities collapse into one [`Selection`] over both ids,
    /// won by the safest operator — the selection mirrors the merge that
    /// `anonymize` applies.
    #[tokio::test]
    async fn select_merges_overlap_into_one_selection() {
        // NAME [0,5) → Replace (Partial); SSN [3,12) → Erase (Irrecoverable).
        let entities = vec![entity("NAME", 0, 5), entity("SSN", 3, 12)];
        let anonymizer = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Replace::default()))
            .with(Rule::label(LabelRef::new("SSN"), Erase));

        let selections = anonymizer.select(&entities, &Scope::default());

        assert_eq!(selections.len(), 1, "overlap collapses to one selection");
        let selection = &selections[0];
        assert_eq!(
            selection.operator_id().name,
            "erase",
            "safest operator wins"
        );
        assert_eq!(selection.entities().len(), 2, "covers both merged entities");
        assert!(selection.entities().contains(&entities[0].id));
        assert!(selection.entities().contains(&entities[1].id));
    }

    /// `anonymize_selections` applies a set of selections, running each
    /// selection's live operator and recording the redaction on every entity
    /// it covers.
    #[tokio::test]
    async fn anonymize_selections_applies_the_picks() {
        let mut target = StrReader("alice and bob".to_owned());
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        let selections = anonymizer.select(&entities, &Scope::default());
        anonymizer
            .anonymize_selections(&mut target, &mut entities, &selections)
            .await
            .unwrap();
        // Both selections applied; each covered entity recorded the redaction.
        for entity in &entities {
            assert!(
                entity
                    .audit
                    .events()
                    .iter()
                    .any(|e| matches!(&e.kind, AuditKind::Redaction { .. })),
                "each covered entity records its redaction",
            );
        }
    }

    /// Swapping a selection's operator before apply overrides the pick — the
    /// mechanism a round-tripped, reviewer-edited selection rides on (rebuild
    /// via [`Selection::new`] with a different operator). The recorded
    /// provenance reflects the operator actually run.
    #[tokio::test]
    async fn overriding_a_selections_operator_changes_the_redaction() {
        let mut target = StrReader("alice".to_owned());
        let mut entities = vec![entity("NAME", 0, 5)];
        // Selection picks Replace...
        let anonymizer =
            Anonymizer::new().with(Rule::label(LabelRef::new("NAME"), Replace::default()));
        let selected = anonymizer.select(&entities, &Scope::default());
        assert_eq!(selected[0].operator_id().name, "replace");

        // ...but the caller rebuilds it to run Erase instead.
        let overridden: Vec<_> = selected
            .iter()
            .map(|s| {
                Selection::new(
                    Arc::new(Erase) as Arc<dyn Operator<Text>>,
                    s.entities().to_vec(),
                    s.matched_by().clone(),
                    s.attribution().cloned(),
                )
            })
            .collect();
        anonymizer
            .anonymize_selections(&mut target, &mut entities, &overridden)
            .await
            .unwrap();

        let ran_erase = entities[0].audit.events().iter().any(|e| {
            matches!(&e.kind, AuditKind::Redaction { operator, .. } if operator.name == "erase")
        });
        assert!(
            ran_erase,
            "provenance records the overriding operator, not the selected one"
        );
    }

    /// A selection's [`view`](Selection::view) is a plain-data round-trip of
    /// the pick: the same operator id, covered entities, matched rule, and
    /// attribution, minus the live operator.
    #[test]
    fn view_mirrors_the_selection() {
        let entities = vec![entity("NAME", 0, 5)];
        let anonymizer =
            Anonymizer::new().with(Rule::label(LabelRef::new("NAME"), Replace::default()));
        let selections = anonymizer.select(&entities, &Scope::default());

        let view = selections[0].view();
        assert_eq!(view.operator_id, selections[0].operator_id());
        assert_eq!(view.entities, selections[0].entities());
        assert_eq!(&view.matched_by, selections[0].matched_by());
    }
}
