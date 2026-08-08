//! The [`Anonymizer`] — the "hide" engine.
//!
//! The redaction counterpart to [`Analyzer`]: an ordered list of
//! selection rules plus its entry points. [`select`] resolves each
//! entity's operator into a reviewable [`Selection`] without touching the
//! data; [`anonymize`] selects, computes each [`Replacement`], and applies
//! the batch back into the target in one step; [`plan`] stops a step short
//! and hands back the [`Redactions`] batch for inspection or deferred
//! application; [`anonymize_selections`] applies a (possibly reviewed) set
//! of selections whose operators a caller-supplied resolver rebuilds.
//!
//! [`Analyzer`]: crate::Analyzer
//! [`select`]: Anonymizer::select
//! [`anonymize`]: Anonymizer::anonymize
//! [`anonymize_selections`]: Anonymizer::anonymize_selections
//! [`plan`]: Anonymizer::plan
//! [`Replacement`]: elide_core::modality::Modality::Replacement

mod registry;
mod rule;
mod selection;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::provenance::Event;
use elide_core::entity::{Entity, LabelCatalog};
use elide_core::modality::{DataReader, DataWriter, Modality, ModalityLocation};
use elide_core::operator::{Operator, Redactions};

pub use self::rule::Rule;
pub use self::selection::Selection;
use self::registry::OperatorRegistry;

/// An operator stored in a [`Rule`], type-erased and shared.
pub(crate) type SharedOperator<M> = Arc<dyn Operator<M>>;

/// Boxed predicate over an entity, used by [`Matcher::Predicate`].
///
/// Receives the [`LabelCatalog`] (empty when none was set) so a predicate
/// can ask catalog-level questions — a label's tags or metadata — the same
/// way a [`Matcher::Tag`] resolves through it.
pub(crate) type Predicate<M> = Box<dyn Fn(&Entity<M>, &LabelCatalog) -> bool + Send + Sync>;

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
///     .with(Rule::predicate(|e| !ConfidenceThreshold::BASELINE.passes(e.confidence), Keep))
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
/// [`Attribution`]: elide_core::entity::provenance::Attribution
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
    /// let attr = Attribution::new("hipaa-safe-harbor");
    /// Anonymizer::new().with_multiple([
    ///     Rule::label(DATE_OF_BIRTH, GeneralizeDate::new(Year)).because(attr.clone()),
    ///     Rule::label(AGE, Clamp::new().with_ceiling(90.0, "90 or older")).because(attr.clone()),
    ///     Rule::label(PERSON_NAME, Erase).because(attr),
    /// ]);
    /// ```
    ///
    /// [`Attribution`]: elide_core::entity::provenance::Attribution
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
    /// Entities whose label has no operator and no fallback are skipped — no
    /// `Selection` is emitted for them.
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
    pub fn select(&self, entities: &[Entity<M>]) -> Vec<Selection<M>> {
        let mut selections = Vec::new();
        for cluster in cluster_overlaps(entities) {
            // Pick the safest operator in the cluster — the one that leaks
            // least — to redact the whole overlapping span; ties go to the
            // wider span, then the earlier position. A singleton cluster just
            // resolves its one entity. `None` means no member had an operator.
            let Some((operator, matched_by, attribution)) = cluster
                .iter()
                .copied()
                .filter_map(|i| self.operators.resolve(&entities[i]).map(|r| (i, r)))
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

    /// Plan the redaction for every entity, reading each one's value from
    /// `reader`, without applying anything.
    ///
    /// The composition of [`select`] (resolve the operator per redaction)
    /// and reading + running each operator to produce its replacement, minus
    /// the final write. Entities whose location reads no data are skipped.
    /// Every entity a redaction covers records a [`Redaction`] event on its
    /// provenance, so the report stays faithful even for entities that were
    /// merged into another's span.
    ///
    /// Returns the [`Redactions`] batch — inspect, serialize, or audit it,
    /// then apply it yourself, or call [`anonymize`] to plan and apply in
    /// one step.
    ///
    /// [`select`]: Self::select
    /// [`anonymize`]: Self::anonymize
    /// [`Redaction`]: elide_core::entity::provenance::EventKind::Redaction
    pub async fn plan(
        &self,
        entities: &mut [Entity<M>],
        reader: &impl DataReader<M>,
    ) -> Result<Redactions<M>> {
        let selections = self.select(entities);
        self.execute(entities, &selections, reader, |s| Ok(Arc::clone(s.operator())))
            .await
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
    /// Use [`select`] + [`anonymize_selections`] instead when the picks must
    /// be reviewed (or round-tripped) between selection and application, or
    /// [`plan`] when you need the [`Redactions`] batch without applying it.
    ///
    /// [`select`]: Self::select
    /// [`anonymize_selections`]: Self::anonymize_selections
    /// [`plan`]: Self::plan
    pub async fn anonymize<T>(&self, target: &mut T, entities: &mut [Entity<M>]) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
    {
        let selections = self.select(entities);
        let redactions = self
            .execute(entities, &selections, target, |s| Ok(Arc::clone(s.operator())))
            .await?;
        target.write_at(redactions).await
    }

    /// Apply a set of (possibly reviewed) [`Selection`]s back into `target`.
    ///
    /// The apply half of the reviewable path: where [`select`] produced the
    /// picks and a reviewer may have edited them, this runs each selection's
    /// operator and writes the result. Because a round-tripped selection has
    /// lost its live operator (a trait object does not survive
    /// serialization), `resolve` rebuilds the operator for each selection —
    /// typically from its [`operator_id`] and config through an operator
    /// registry the caller wired with any runtime capabilities (keys, a
    /// vault). For the in-process path, `resolve` can simply hand back the
    /// operator the selection still carries.
    ///
    /// Every entity a selection covers records a [`Redaction`] event, so
    /// provenance stays faithful for merged entities too.
    ///
    /// [`select`]: Self::select
    /// [`operator_id`]: Selection::operator_id
    /// [`Redaction`]: elide_core::entity::provenance::EventKind::Redaction
    pub async fn anonymize_selections<T, R>(
        &self,
        target: &mut T,
        entities: &mut [Entity<M>],
        selections: &[Selection<M>],
        resolve: R,
    ) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
        R: Fn(&Selection<M>) -> Result<Arc<dyn Operator<M>>>,
    {
        let redactions = self.execute(entities, selections, target, resolve).await?;
        target.write_at(redactions).await
    }

    /// Read, run, and record each selection, returning the [`Redactions`]
    /// batch without writing it — the shared core of [`plan`], [`anonymize`],
    /// and [`anonymize_selections`].
    ///
    /// For each selection: rebuild its operator via `resolve`, union the
    /// spans of the entities it covers, read that span, run the operator, and
    /// record a [`Redaction`] event on every covered entity. Selections whose
    /// span reads no data are skipped.
    ///
    /// [`plan`]: Self::plan
    /// [`anonymize`]: Self::anonymize
    /// [`anonymize_selections`]: Self::anonymize_selections
    /// [`Redaction`]: elide_core::entity::provenance::EventKind::Redaction
    async fn execute<R>(
        &self,
        entities: &mut [Entity<M>],
        selections: &[Selection<M>],
        reader: &impl DataReader<M>,
        resolve: R,
    ) -> Result<Redactions<M>>
    where
        R: Fn(&Selection<M>) -> Result<Arc<dyn Operator<M>>>,
    {
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
            let operator = resolve(selection)?;

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
                let event = Event::redaction(
                    operator.id(),
                    operator.leak_profile(),
                    entity.confidence,
                    selection.matched_by().clone(),
                    selection.attribution().cloned(),
                );
                entity.provenance.record(event);
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
    use elide_core::entity::provenance::{Event, EventKind, PatternEvent, Provenance};
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::operators::{Erase, Replace};

    /// In-memory text reader: slices the backing string by byte range. Also
    /// a no-op [`DataWriter`] so the apply-path entry points that write can be
    /// exercised without a full codec document.
    struct StrReader(String);

    #[async_trait::async_trait]
    impl DataReader<Text> for StrReader {
        async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
            Ok(self.0.get(location.start..location.end).map(TextData::new))
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
        let event = Event::pattern("t", confidence, loc.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new(label),
            loc,
            confidence,
            Provenance::new(event),
        )
    }

    /// Disjoint entities each redact separately — the baseline behaviour.
    #[tokio::test]
    async fn disjoint_entities_redact_separately() {
        let reader = StrReader("alice and bob".to_owned());
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let plan = Anonymizer::new()
            .with(Rule::fallback(Replace::default()))
            .plan(&mut entities, &reader)
            .await
            .unwrap();
        assert_eq!(plan.len(), 2, "two disjoint redactions");
    }

    /// Overlapping entities collapse to one redaction over the union span,
    /// run by the safest (least-leaky) operator. `Erase` (Irrecoverable)
    /// beats `Replace` (Partial).
    #[tokio::test]
    async fn overlap_merges_under_safest_operator() {
        let reader = StrReader("0123456789abc".to_owned());
        // NAME [0,5) → Replace (Partial); SSN [3,12) → Erase (Irrecoverable).
        let mut entities = vec![entity("NAME", 0, 5), entity("SSN", 3, 12)];
        let plan = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Replace::default()))
            .with(Rule::label(LabelRef::new("SSN"), Erase))
            .plan(&mut entities, &reader)
            .await
            .unwrap();

        // One redaction over the union [0,12), by Erase → Removed.
        assert_eq!(plan.len(), 1, "overlap collapses to one redaction");
        let (location, replacement) = plan.iter().next().unwrap();
        assert_eq!((location.start, location.end), (0, 12), "covers the union");
        assert_eq!(replacement.value(), None, "Erase removes, not substitutes");

        // Both entities record a redaction by the winning operator.
        for entity in &entities {
            let redacted = entity.provenance.events.iter().any(|e| {
                matches!(&e.kind, EventKind::Redaction { operator, .. } if operator.name == "erase")
            });
            assert!(redacted, "every member records the erase redaction");
        }
    }

    /// A transitive chain (A–B overlap, B–C overlap, A–C disjoint) still
    /// collapses to one redaction spanning all three.
    #[tokio::test]
    async fn transitive_overlap_chain_merges() {
        let reader = StrReader("0123456789abcdef".to_owned());
        let mut entities = vec![entity("A", 0, 5), entity("B", 4, 9), entity("C", 8, 13)];
        let plan = Anonymizer::new()
            .with(Rule::fallback(Erase))
            .plan(&mut entities, &reader)
            .await
            .unwrap();
        assert_eq!(plan.len(), 1, "the chain collapses to one redaction");
        let (location, _) = plan.iter().next().unwrap();
        assert_eq!((location.start, location.end), (0, 13));
    }

    /// Two entities that overlap by byte range but sit on different pages
    /// can't coalesce into one span, so they stay separate: each redacts on
    /// its own and neither is dropped.
    #[tokio::test]
    async fn non_coalescible_overlap_stays_separate() {
        let reader = StrReader("0123456789".to_owned());
        // Same range, different page: overlaps() is true (page is ignored)
        // but union() is None, so clustering must keep them apart.
        let mut a = entity("A", 0, 5);
        a.location.page = Some(1);
        let mut b = entity("B", 0, 5);
        b.location.page = Some(2);
        let mut entities = vec![a, b];

        let plan = Anonymizer::new()
            .with(Rule::fallback(Erase))
            .plan(&mut entities, &reader)
            .await
            .unwrap();

        assert_eq!(plan.len(), 2, "different pages redact separately");
        // Neither entity is silently dropped — both record a redaction.
        for entity in &entities {
            assert!(
                entity
                    .provenance
                    .events
                    .iter()
                    .any(|e| matches!(&e.kind, EventKind::Redaction { .. })),
                "every entity records its own redaction",
            );
        }
    }

    /// A trait object built dynamically (as a policy layer would from
    /// config) flows straight into the builder: `Operator` is implemented
    /// for `Box<dyn Operator>` and `Arc<dyn Operator>`, so neither needs
    /// unwrapping to a concrete type first.
    #[tokio::test]
    async fn boxed_and_arced_trait_objects_are_operators() {
        use std::sync::Arc;

        let reader = StrReader("alice bob".to_owned());

        let boxed: Box<dyn Operator<Text>> = Box::new(Replace::default());
        let arced: Arc<dyn Operator<Text>> = Arc::new(Erase);

        let mut entities = vec![entity("NAME", 0, 5), entity("SECRET", 6, 9)];
        let plan = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), boxed))
            .with(Rule::label(LabelRef::new("SECRET"), arced))
            .plan(&mut entities, &reader)
            .await
            .unwrap();

        assert_eq!(plan.len(), 2, "both trait-object operators ran");
    }

    /// `select` resolves one [`Selection`] per redaction, naming the winning
    /// operator and the entities it covers — without reading any data.
    #[tokio::test]
    async fn select_resolves_one_selection_per_redaction() {
        let entities = vec![entity("NAME", 0, 5), entity("URL", 10, 13)];
        let anonymizer = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Erase))
            .with(Rule::label(LabelRef::new("URL"), Replace::default()));

        let selections = anonymizer.select(&entities);

        assert_eq!(selections.len(), 2, "one selection per disjoint entity");
        // Each selection covers exactly its own entity, by id.
        for (selection, entity) in selections.iter().zip(&entities) {
            assert_eq!(selection.entities(), [entity.id]);
        }
    }

    /// Overlapping entities collapse into one [`Selection`] over both ids,
    /// won by the safest operator — the selection mirrors the merge that
    /// `plan`/`anonymize` apply.
    #[tokio::test]
    async fn select_merges_overlap_into_one_selection() {
        // NAME [0,5) → Replace (Partial); SSN [3,12) → Erase (Irrecoverable).
        let entities = vec![entity("NAME", 0, 5), entity("SSN", 3, 12)];
        let anonymizer = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Replace::default()))
            .with(Rule::label(LabelRef::new("SSN"), Erase));

        let selections = anonymizer.select(&entities);

        assert_eq!(selections.len(), 1, "overlap collapses to one selection");
        let selection = &selections[0];
        assert_eq!(selection.operator_id().name, "erase", "safest operator wins");
        assert_eq!(selection.entities().len(), 2, "covers both merged entities");
        assert!(selection.entities().contains(&entities[0].id));
        assert!(selection.entities().contains(&entities[1].id));
    }

    /// `anonymize_selections` applies a set of selections, rebuilding each
    /// operator through the caller's resolver — here the in-process case,
    /// where the resolver hands back the live operator the selection carries.
    #[tokio::test]
    async fn anonymize_selections_applies_via_resolver() {
        let mut target = StrReader("alice and bob".to_owned());
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        let selections = anonymizer.select(&entities);
        anonymizer
            .anonymize_selections(&mut target, &mut entities, &selections, |s| {
                Ok(Arc::clone(s.operator()))
            })
            .await
            .unwrap();
        // Both selections applied; each covered entity recorded the redaction
        // the resolver ran.
        for entity in &entities {
            assert!(
                entity
                    .provenance
                    .events
                    .iter()
                    .any(|e| matches!(&e.kind, EventKind::Redaction { .. })),
                "each covered entity records its redaction",
            );
        }
    }

    /// A resolver may substitute a *different* operator than the one selected
    /// — the mechanism a round-tripped, reviewer-edited selection rides on.
    /// The recorded provenance reflects the operator the resolver returned.
    #[tokio::test]
    async fn resolver_substitutes_the_operator() {
        let mut target = StrReader("alice".to_owned());
        let mut entities = vec![entity("NAME", 0, 5)];
        // Selection picks Replace...
        let anonymizer = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Replace::default()));
        let selections = anonymizer.select(&entities);
        assert_eq!(selections[0].operator_id().name, "replace");

        // ...but the resolver runs Erase instead.
        anonymizer
            .anonymize_selections(&mut target, &mut entities, &selections, |_s| {
                Ok(Arc::new(Erase) as Arc<dyn Operator<Text>>)
            })
            .await
            .unwrap();

        let ran_erase = entities[0].provenance.events.iter().any(|e| {
            matches!(&e.kind, EventKind::Redaction { operator, .. } if operator.name == "erase")
        });
        assert!(ran_erase, "provenance records the resolver's operator, not the selected one");
    }
}
