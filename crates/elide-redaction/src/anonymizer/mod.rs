//! The [`Anonymizer`] — the "hide" engine.
//!
//! The redaction counterpart to [`Analyzer`]: an ordered list of
//! selection rules plus two entry points. [`anonymize`] picks each
//! entity's operator, computes its [`Replacement`], and applies the batch
//! back into the target in one step; [`plan`] stops a step short and
//! hands back the [`Redactions`] batch for inspection or deferred
//! application.
//!
//! [`Analyzer`]: crate::Analyzer
//! [`anonymize`]: Anonymizer::anonymize
//! [`plan`]: Anonymizer::plan
//! [`Replacement`]: elide_core::modality::Modality::Replacement

mod dyn_operator;
mod registry;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::provenance::{Attribution, Event};
use elide_core::entity::{Entity, LabelCatalog, LabelRef};
use elide_core::modality::{DataReader, DataWriter, Modality, ModalityLocation};
use elide_core::operator::{Operator, Redactions};
use hipstr::HipStr;

use self::registry::{Matcher, OperatorRegistry};

/// The hide engine: selects an operator per entity and computes its
/// replacement.
///
/// Generic over the [`Modality`] `M`. Selection is an *ordered list of
/// rules*, tried top to bottom with the first match winning: bind an
/// operator to an exact label with [`with_label`], to a label tag with
/// [`with_tag`] (which needs a catalog, see [`with_catalog`]), to an
/// arbitrary predicate with [`with_predicate`], or as a catch-all with
/// [`with_fallback`]. [`anonymize`] resolves and runs the operators,
/// applying the replacements back into the target.
///
/// ```ignore
/// Anonymizer::new()
///     .with_catalog(LabelCatalog::with_builtins())
///     // Order matters: a weak detection is kept as-is before any
///     // label or tag rule can fire.
///     .with_predicate(|e| !ConfidenceThreshold::BASELINE.passes(e.confidence), Keep)
///     .with_label(LabelRef::new("EMAIL_ADDRESS"), Replace::default())
///     .with_tag("financial", Mask::stars())
///     .with_fallback(Erase)
///     .anonymize(&mut document, &entities)
///     .await?;
/// ```
///
/// [`with_label`]: Anonymizer::with_label
/// [`with_tag`]: Anonymizer::with_tag
/// [`with_predicate`]: Anonymizer::with_predicate
/// [`with_fallback`]: Anonymizer::with_fallback
/// [`with_catalog`]: Anonymizer::with_catalog
/// [`anonymize`]: Anonymizer::anonymize
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

    /// Set the [`LabelCatalog`] that [`with_tag`] rules resolve label
    /// names against. Without it, tag rules never match.
    ///
    /// [`with_tag`]: Self::with_tag
    #[must_use]
    pub fn with_catalog(mut self, catalog: LabelCatalog) -> Self {
        self.operators.set_catalog(catalog);
        self
    }

    /// Append a rule binding `operator` to an exact label.
    #[must_use]
    pub fn with_label<O: Operator<M> + 'static>(mut self, label: LabelRef, operator: O) -> Self {
        self.operators.push(Matcher::Label(label), operator);
        self
    }

    /// Append a rule binding `operator` to every entity whose label
    /// carries `tag`. Requires a catalog set via [`with_catalog`].
    ///
    /// [`with_catalog`]: Self::with_catalog
    #[must_use]
    pub fn with_tag<O: Operator<M> + 'static>(
        mut self,
        tag: impl Into<HipStr<'static>>,
        operator: O,
    ) -> Self {
        self.operators.push(Matcher::Tag(tag.into()), operator);
        self
    }

    /// Append a rule binding `operator` to every entity the `predicate`
    /// accepts. The predicate sees the entity's label, confidence,
    /// location, and provenance.
    ///
    /// Use [`with_catalog_predicate`] when the predicate also needs the
    /// [`LabelCatalog`] (to resolve the entity's label to its tags or
    /// metadata).
    ///
    /// [`with_catalog_predicate`]: Self::with_catalog_predicate
    #[must_use]
    pub fn with_predicate<O, P>(mut self, predicate: P, operator: O) -> Self
    where
        O: Operator<M> + 'static,
        P: Fn(&Entity<M>) -> bool + Send + Sync + 'static,
    {
        self.operators.push(
            Matcher::Predicate(Box::new(move |e, _| predicate(e))),
            operator,
        );
        self
    }

    /// Append a rule binding `operator` to every entity the `predicate`
    /// accepts, where the predicate also receives the [`LabelCatalog`] —
    /// empty when none was set — so it can resolve the entity's label to its
    /// tags or metadata, the same source [`with_tag`] consults.
    ///
    /// The catalog-aware counterpart to [`with_predicate`].
    ///
    /// [`LabelCatalog`]: elide_core::entity::LabelCatalog
    /// [`with_tag`]: Self::with_tag
    /// [`with_predicate`]: Self::with_predicate
    #[must_use]
    pub fn with_catalog_predicate<O, P>(mut self, predicate: P, operator: O) -> Self
    where
        O: Operator<M> + 'static,
        P: Fn(&Entity<M>, &LabelCatalog) -> bool + Send + Sync + 'static,
    {
        self.operators
            .push(Matcher::Predicate(Box::new(predicate)), operator);
        self
    }

    /// Append a catch-all rule: `operator` runs for every entity not
    /// matched by an earlier rule. Equivalent to a predicate that always
    /// accepts, so any rule after it is unreachable.
    #[must_use]
    pub fn with_fallback<O: Operator<M> + 'static>(mut self, operator: O) -> Self {
        self.operators.push(Matcher::Always, operator);
        self
    }

    /// Attribute the most-recently-added rule to a policy: the [`Attribution`]
    /// (a bare policy id, or one built with a reason) is recorded on the
    /// redaction provenance of every entity this rule redacts, the *why*
    /// alongside the matched rule.
    ///
    /// Chains onto a rule builder; a no-op if no rule has been added yet:
    ///
    /// ```ignore
    /// Anonymizer::new()
    ///     .with_label(EMAIL, Replace::default()).because("gdpr-art-17")
    ///     .with_tag("financial", Mask::stars())
    ///         .because(Attribution::new("pci-dss-3.4").with_reason("PAN masking"));
    /// ```
    ///
    /// [`Attribution`]: elide_core::entity::provenance::Attribution
    #[must_use]
    pub fn because(mut self, attribution: impl Into<Attribution>) -> Self {
        self.operators.set_last_attribution(attribution.into());
        self
    }

    /// Plan the redaction for every entity, reading each one's value from
    /// `reader`, without applying anything.
    ///
    /// For each entity: resolve the operator for its label (its mapping,
    /// else the fallback), read the entity's value via
    /// [`DataReader::read_at`], and run the operator to produce a
    /// replacement. Entities whose label has no operator and no fallback
    /// are skipped, as are entities whose location reads no data.
    ///
    /// **Overlapping entities are merged.** Where a set of entities overlap
    /// in the medium (a left-over nesting, or one a user re-introduced by
    /// editing the report), redacting each separately would write competing
    /// operators over the same bytes and corrupt the output. Instead the
    /// overlapping set collapses to *one* redaction covering the
    /// [union][union] of their spans, run by the **safest** operator among
    /// them — the one whose output leaks least (highest [`LeakProfile`]).
    /// Ties go to the wider span, then the earlier position. The absorbed
    /// entities still record a redaction event noting they were merged, so
    /// the report stays faithful. A purely mechanical safety step: it makes
    /// no semantic choice about which *finding* is right — that is
    /// detection's job.
    ///
    /// Returns the [`Redactions`] batch — inspect, serialize, or audit it,
    /// then apply it yourself, or call [`anonymize`] to plan and apply in
    /// one step.
    ///
    /// [`anonymize`]: Self::anonymize
    /// [union]: elide_core::modality::ModalityLocation::union
    /// [`LeakProfile`]: elide_core::operator::LeakProfile
    pub async fn plan(
        &self,
        entities: &mut [Entity<M>],
        reader: &impl DataReader<M>,
    ) -> Result<Redactions<M>> {
        let mut redactions = Redactions::new();
        for cluster in cluster_overlaps(entities) {
            // Pick the safest operator in the cluster — the one that leaks
            // least — to redact the whole overlapping span; ties go to the
            // wider span, then the earlier position. A singleton cluster just
            // resolves its one entity. `None` means no member had an operator.
            let Some((winner, operator, matched_by, attribution)) = cluster
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
                .map(|(i, r)| {
                    (
                        i,
                        Arc::clone(r.operator),
                        r.matched_by,
                        r.attribution.cloned(),
                    )
                })
            else {
                continue;
            };

            // Redact the union of every member's span. Clustering groups only
            // entities that coalesce, so the fold never hits `None`; a
            // singleton unions to itself.
            let location = cluster
                .iter()
                .map(|&i| entities[i].location.clone())
                .reduce(|acc, loc| {
                    acc.union(&loc)
                        .expect("cluster members coalesce by construction")
                })
                .expect("a cluster is never empty");
            let Some(data) = reader.read_at(&location).await? else {
                tracing::debug!(modality = M::NAME, "location read no data; skipping");
                continue;
            };
            let replacement = operator.anonymize_boxed(&entities[winner], &data).await?;

            // Record the redaction on every member, so each entity's
            // provenance reflects that this operator hid it.
            for &i in &cluster {
                let entity = &mut entities[i];
                let event = Event::redaction(
                    operator.id(),
                    operator.leak_profile(),
                    entity.confidence,
                    matched_by.clone(),
                    attribution.clone(),
                );
                entity.provenance.record(event);
            }
            redactions.push(location, replacement);
        }
        Ok(redactions)
    }

    /// Hide every entity by applying its operator's replacement back into
    /// `target`.
    ///
    /// The complete redaction step: [`plan`]s each entity's replacement
    /// (reading its value from `target`), then hands the batch to
    /// [`DataWriter::write_at`] so `target` owns the *how* and *ordering*
    /// of applying it. `target` is both the reader and the writer —
    /// typically a decoded codec document. Entities must already be in
    /// `target`'s coordinate system.
    ///
    /// Use [`plan`] instead when you need the [`Redactions`] batch before
    /// (or instead of) applying it.
    ///
    /// [`plan`]: Self::plan
    pub async fn anonymize<T>(&self, target: &mut T, entities: &mut [Entity<M>]) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
    {
        let redactions = self.plan(entities, target).await?;
        target.write_at(redactions).await
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
    use elide_core::entity::provenance::{Event, EventKind, PatternEvent, Provenance};
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::operators::{Erase, Replace};

    /// In-memory text reader: slices the backing string by byte range.
    struct StrReader(String);

    impl DataReader<Text> for StrReader {
        async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
            Ok(self.0.get(location.start..location.end).map(TextData::new))
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
            .with_fallback(Replace::default())
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
            .with_label(LabelRef::new("NAME"), Replace::default())
            .with_label(LabelRef::new("SSN"), Erase)
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
            .with_fallback(Erase)
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
            .with_fallback(Erase)
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
}
