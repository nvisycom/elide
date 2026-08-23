//! The [`Anonymizer`] — the "hide" engine.
//!
//! The redaction counterpart to [`Analyzer`]: an ordered list of selection
//! rules plus its entry points. [`pick`] resolves each entity's operator and
//! records the decision as a [`Selection`] event on the entity's audit trail,
//! without touching the data, so the picks can be reviewed (and the entities
//! edited) first; [`redact`] re-resolves the configured operator, computes each
//! [`Replacement`], and applies the batch back into the target; [`anonymize`]
//! does both in one step.
//!
//! [`Analyzer`]: crate::Analyzer
//! [`pick`]: Anonymizer::pick
//! [`redact`]: Anonymizer::redact
//! [`anonymize`]: Anonymizer::anonymize
//! [`Selection`]: elide_core::entity::audit::AuditKind::Selection
//! [`Replacement`]: elide_core::modality::Modality::Replacement

mod registry;
mod rule;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::audit::{Attribution, AuditEvent, RuleMatch};
use elide_core::entity::{Entity, LabelCatalog};
use elide_core::modality::{DataReader, DataWriter, Modality, ModalityLocation};
use elide_core::operator::{Operator, Redactions};
use elide_core::recognition::Scope;

use self::registry::OperatorRegistry;
pub use self::rule::{MatchContext, Rule};

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

    /// Record the operator *pick* for every entity onto its audit trail,
    /// without reading any data — the *decision* half of redaction, split out
    /// so the picks can be inspected (and the entities edited) before anything
    /// is applied.
    ///
    /// For each overlap cluster it resolves the winning operator (see the merge
    /// rule below) and records a [`Selection`] event — naming the operator, the
    /// rule that matched, and any policy attribution — on **every** entity the
    /// cluster covers, so a merged non-winner still shows why it will be hidden.
    /// No document data is touched. Entities whose label has no operator and no
    /// fallback get no pick; a reviewer-suppressed entity is skipped entirely.
    ///
    /// `scope` is the caller-asserted request [`Scope`], passed to
    /// [predicate rules](Rule::predicate) via [`MatchContext::scope`] so the pick
    /// can branch on request context.
    ///
    /// [`Scope`]: elide_core::recognition::Scope
    /// [`Selection`]: elide_core::entity::audit::AuditKind::Selection
    ///
    /// **Overlapping entities are merged.** Where a set of entities overlap in
    /// the medium (a left-over nesting, or one a user re-introduced by editing
    /// the report), redacting each separately would write competing operators
    /// over the same bytes and corrupt the output. Instead the overlapping set
    /// resolves to *one* operator — the **safest** among them (the one whose
    /// output leaks least, highest [`LeakProfile`]); ties go to the wider span,
    /// then the earlier position — which then covers the union of their spans.
    /// A purely mechanical safety step: it makes no semantic choice about which
    /// *finding* is right — that is detection's job.
    ///
    /// [`anonymize`] runs `pick` and applies in one step.
    ///
    /// [`anonymize`]: Self::anonymize
    /// [`LeakProfile`]: elide_core::operator::LeakProfile
    pub fn pick(&self, entities: &mut [Entity<M>], scope: &Scope) {
        for cluster in self.resolve_clusters(entities, scope) {
            // Record the same pick on every covered entity. Built per member
            // (rather than cloned) because `AuditEvent<M>: Clone` would demand
            // `M: Clone`, which a modality marker need not be.
            for &i in &cluster.members {
                let event = AuditEvent::selection(
                    cluster.operator.id(),
                    entities[i].confidence,
                    cluster.matched_by.clone(),
                    cluster.attribution.clone(),
                );
                entities[i].audit.record(event);
            }
        }
    }

    /// Resolve each overlap cluster to its winning operator, dropping
    /// reviewer-suppressed entities and clusters no rule covers — the shared
    /// decision behind [`pick`](Self::pick) and [`redact`](Self::redact).
    ///
    /// Operators are cloned out of the rule registry (config intact) so the
    /// borrow on `self` ends and the caller can mutate `entities` freely.
    fn resolve_clusters(&self, entities: &[Entity<M>], scope: &Scope) -> Vec<ResolvedCluster<M>> {
        let mut resolved = Vec::new();
        for cluster in cluster_overlaps(entities) {
            // A reviewer-suppressed entity is left alone: it contributes no
            // operator and is not covered, so it is never hidden even when it
            // overlaps a live entity. Filter it out of the cluster first.
            let members: Vec<usize> = cluster
                .iter()
                .copied()
                .filter(|&i| !entities[i].is_suppressed())
                .collect();
            // Pick the safest operator among the live members — the one that
            // leaks least; ties go to the wider span, then the earlier
            // position. `None` means no live member had an operator (or every
            // member was suppressed).
            let Some((operator, matched_by, attribution)) = members
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
            resolved.push(ResolvedCluster {
                members,
                operator,
                matched_by,
                attribution,
            });
        }
        resolved
    }

    /// Hide every entity by applying its operator's replacement back into
    /// `target`.
    ///
    /// The complete redaction step: [`pick`]s each entity's operator,
    /// reads its value from `target`, runs the operator, and hands the batch
    /// to [`DataWriter::write_at`] so `target` owns the *how* and *ordering*
    /// of applying it. `target` is both the reader and the writer —
    /// typically a decoded codec document. Entities must already be in
    /// `target`'s coordinate system.
    ///
    /// `scope` is passed to the operator rules for scope-aware selection.
    ///
    /// Use [`pick`] then [`redact`] instead when the picks must be reviewed
    /// (each recorded on its entity's audit trail) between decision and apply.
    ///
    /// [`pick`]: Self::pick
    /// [`redact`]: Self::redact
    pub async fn anonymize<T>(
        &self,
        target: &mut T,
        entities: &mut [Entity<M>],
        scope: &Scope,
    ) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
    {
        self.pick(entities, scope);
        self.redact(target, entities, scope).await
    }

    /// Apply the redaction back into `target`: for each overlap cluster,
    /// re-resolve its winning operator from the rules, read the union span, run
    /// the operator, and record a [`Redaction`] event on every covered entity.
    ///
    /// This is the apply half of the reviewable path. Picks live on the
    /// entities' audit trails ([`pick`]); the operator's *configuration* lives
    /// in the rules, so `redact` re-resolves the live configured operator here
    /// rather than reading it from the audit — a reviewer overrides by editing
    /// the entity (suppress, retag), not by rewriting the pick. A
    /// reviewer-suppressed entity is left alone. `scope` selects the same rules
    /// [`pick`] used.
    ///
    /// [`pick`]: Self::pick
    /// [`Redaction`]: elide_core::entity::audit::AuditKind::Redaction
    pub async fn redact<T>(
        &self,
        target: &mut T,
        entities: &mut [Entity<M>],
        scope: &Scope,
    ) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
    {
        let clusters = self.resolve_clusters(entities, scope);
        let mut redactions = Redactions::new();
        for cluster in clusters {
            let winner = cluster.members[0];
            // Redact the union of every member's span. Clustering groups only
            // entities that coalesce, so the fold never hits `None`; a
            // singleton unions to itself.
            let location = cluster
                .members
                .iter()
                .map(|&i| entities[i].location.clone())
                .reduce(|acc, loc| {
                    acc.union(&loc)
                        .expect("cluster members coalesce by construction")
                })
                .expect("a cluster covers at least one entity");
            let Some(data) = target.read_at(&location).await? else {
                tracing::debug!(modality = M::NAME, "location read no data; skipping");
                continue;
            };
            let replacement = cluster.operator.anonymize(&entities[winner], &data).await?;

            // Record the redaction on every member, so each entity's
            // provenance reflects that this operator hid it.
            for &i in &cluster.members {
                let event = AuditEvent::redaction(
                    cluster.operator.id(),
                    cluster.operator.leak_profile(),
                    entities[i].confidence,
                    cluster.matched_by.clone(),
                    cluster.attribution.clone(),
                );
                entities[i].audit.record(event);
            }
            redactions.push(location, replacement);
        }
        target.write_at(redactions).await
    }
}

/// One overlap cluster resolved to the operator that will hide it: the covered
/// entity indices (winner first), the live configured operator (cloned from the
/// rules), and the pick's provenance. The shared output of [`Anonymizer::pick`]
/// and [`Anonymizer::redact`].
struct ResolvedCluster<M: Modality> {
    members: Vec<usize>,
    operator: SharedOperator<M>,
    matched_by: RuleMatch,
    attribution: Option<Attribution>,
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

    /// The [`Selection`] pick recorded on an entity, if any.
    ///
    /// [`Selection`]: AuditKind::Selection
    fn pick_of(entity: &Entity<Text>) -> Option<&AuditKind<Text>> {
        entity
            .audit
            .events()
            .iter()
            .map(|e| &e.kind)
            .find(|k| matches!(k, AuditKind::Selection(_)))
    }

    fn is_redacted(entity: &Entity<Text>) -> bool {
        entity
            .audit
            .events()
            .iter()
            .any(|e| matches!(&e.kind, AuditKind::Redaction(_)))
    }

    /// `pick` records one [`Selection`] event per entity, naming the winning
    /// operator — without reading or writing any data.
    #[tokio::test]
    async fn pick_records_one_selection_per_entity() {
        let mut entities = vec![entity("NAME", 0, 5), entity("URL", 10, 13)];
        let anonymizer = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Erase))
            .with(Rule::label(LabelRef::new("URL"), Replace::default()));

        anonymizer.pick(&mut entities, &Scope::default());

        // Each disjoint entity carries its own pick, and nothing is redacted.
        let names: Vec<_> = entities
            .iter()
            .map(|e| match pick_of(e) {
                Some(AuditKind::Selection(s)) => s.operator.name.to_string(),
                _ => panic!("every entity records a Selection pick"),
            })
            .collect();
        assert_eq!(names, ["erase", "replace"]);
        assert!(
            entities.iter().all(|e| !is_redacted(e)),
            "pick never redacts"
        );
    }

    /// A [scope-aware predicate](Rule::predicate) reading [`MatchContext::scope`]
    /// lets the *same* entity be picked differently per request `Scope`. Here an
    /// auditor keeps a wider prefix than a support agent — the pick's
    /// [`matched_by`](AuditKind::Selection) records which rule fired.
    #[tokio::test]
    async fn scope_predicate_picks_per_audience() {
        let anonymizer = Anonymizer::new()
            // Auditors: keep the leading 6. Anyone else: mask everything.
            .with(Rule::predicate(
                |cx| cx.scope.metadata.audience.iter().any(|a| a == "auditor"),
                Mask::stars().with_keep_prefix(6),
            ))
            .with(Rule::fallback(Mask::stars()));

        let mut for_auditor = vec![entity("PAYMENT_CARD", 0, 16)];
        let mut for_support = vec![entity("PAYMENT_CARD", 0, 16)];
        anonymizer.pick(&mut for_auditor, &Scope::new().with_audience(["auditor"]));
        anonymizer.pick(&mut for_support, &Scope::new().with_audience(["support"]));

        // One detection, two scopes → two different rules fire. The predicate
        // rule matched the auditor; the support agent fell through to the
        // fallback.
        assert!(
            matches!(
                pick_of(&for_auditor[0]),
                Some(AuditKind::Selection(s)) if matches!(s.matched_by, RuleMatch::Predicate)
            ),
            "auditor matches the scope predicate",
        );
        assert!(
            matches!(
                pick_of(&for_support[0]),
                Some(AuditKind::Selection(s)) if matches!(s.matched_by, RuleMatch::Fallback)
            ),
            "support falls through to the fallback",
        );
    }

    /// Overlapping entities are won by the safest operator, and the pick is
    /// recorded on *every* member of the cluster — mirroring the merge that
    /// `redact` applies.
    #[tokio::test]
    async fn pick_records_the_cluster_winner_on_every_member() {
        // NAME [0,5) → Replace (Partial); SSN [3,12) → Erase (Irrecoverable).
        let mut entities = vec![entity("NAME", 0, 5), entity("SSN", 3, 12)];
        let anonymizer = Anonymizer::new()
            .with(Rule::label(LabelRef::new("NAME"), Replace::default()))
            .with(Rule::label(LabelRef::new("SSN"), Erase));

        anonymizer.pick(&mut entities, &Scope::default());

        // Both merged entities record the same winning pick: the safest
        // operator, `erase`.
        for entity in &entities {
            match pick_of(entity) {
                Some(AuditKind::Selection(s)) => {
                    assert_eq!(s.operator.name, "erase", "safest operator wins");
                }
                _ => panic!("every cluster member records the pick"),
            }
        }
    }

    /// `redact` applies each cluster's re-resolved live operator and records the
    /// redaction on every entity it covers.
    #[tokio::test]
    async fn redact_applies_the_picks() {
        let mut target = StrReader("alice and bob".to_owned());
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        anonymizer
            .redact(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();
        // Each covered entity recorded the redaction.
        assert!(
            entities.iter().all(is_redacted),
            "each covered entity records its redaction",
        );
    }

    /// `anonymize` does both halves: every entity records a pick *and* a
    /// redaction, and the audit chain still verifies.
    #[tokio::test]
    async fn anonymize_picks_then_redacts() {
        let mut target = StrReader("alice and bob".to_owned());
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        anonymizer
            .anonymize(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();

        for entity in &entities {
            assert!(pick_of(entity).is_some(), "records the pick");
            assert!(is_redacted(entity), "records the redaction");
            assert!(
                entity.audit.verify().is_ok(),
                "the audit chain still verifies"
            );
        }
    }

    /// A reviewer-suppressed entity is left alone: `pick` records no pick for
    /// it, and `redact` records no redaction on it, while a co-located live
    /// entity is still picked and redacted.
    #[tokio::test]
    async fn a_suppressed_entity_is_not_picked_or_redacted() {
        let mut target = StrReader("alice and bob".to_owned());
        let live = entity("NAME", 0, 5);
        let mut suppressed = entity("NAME", 10, 13);
        let suppressed_id = suppressed.id;
        let live_id = live.id;
        // The reviewer marks the second detection as a false positive.
        suppressed.suppress(
            AuditEvent::manual(suppressed.location.clone(), suppressed.confidence)
                .with_reason("false positive"),
        );
        let mut entities = vec![live, suppressed];

        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        anonymizer
            .anonymize(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();

        let of = |id| entities.iter().find(|e| e.id == id).unwrap();
        assert!(pick_of(of(live_id)).is_some(), "the live entity is picked");
        assert!(is_redacted(of(live_id)), "the live entity is redacted");
        assert!(
            pick_of(of(suppressed_id)).is_none(),
            "no pick for the suppressed entity",
        );
        assert!(
            !is_redacted(of(suppressed_id)),
            "the suppressed entity is left alone",
        );
        // The suppression itself is on the trail, with its reason.
        let has_manual = of(suppressed_id).audit.events().iter().any(|e| {
            matches!(&e.kind, AuditKind::Manual(m) if m.reason.as_deref() == Some("false positive"))
        });
        assert!(has_manual, "the suppression is audited with its reason");
    }
}
