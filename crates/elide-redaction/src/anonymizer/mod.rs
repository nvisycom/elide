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
use elide_core::entity::audit::{Attribution, AuditEvent, Redaction, RuleMatch, Selection};
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
    ///     .with(Rule::label(EMAIL, Replace::default()).because(Attribution::freeform("gdpr-art-17")))
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
    /// let attr: Attribution = Attribution::freeform("hipaa-safe-harbor").into();
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
                let mut selection =
                    Selection::new(cluster.operator.id(), cluster.matched_by.clone());
                if let Some(attribution) = cluster.attribution.clone() {
                    selection = selection.with_attribution(attribution);
                }
                let event = AuditEvent::selection(selection, entities[i].confidence);
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
        // A reviewer-suppressed entity is left alone: it contributes no operator
        // and does not extend the redacted span. (Where it overlaps a live
        // entity, the shared bytes still fall inside the live member's own span —
        // span-level redaction cannot spare those.) Exclude suppressed entities
        // from clustering entirely, so a suppressed span can never bridge two
        // live ones into a cluster that fails to coalesce once it is dropped.
        let live: Vec<usize> = (0..entities.len())
            .filter(|&i| !entities[i].is_suppressed())
            .collect();
        let mut resolved = Vec::new();
        for members in cluster_overlaps(entities, &live) {
            // Pick the safest operator among the members — the one that leaks
            // least; ties go to the wider span, then the earlier position.
            // `None` means no member had an operator.
            let Some((winner, operator, matched_by, attribution)) = members
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
            resolved.push(ResolvedCluster {
                members,
                winner,
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
        // Buffer the (entity index, event) pairs and record them only after
        // `write_at` succeeds: the audit trail is the compliance artifact, so a
        // Redaction event must never claim an operator hid an entity while a
        // later error left the document unwritten.
        let mut pending: Vec<(usize, AuditEvent<M>)> = Vec::new();
        for cluster in clusters {
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
            // Run the operator against the *winning* entity — the one the
            // operator was resolved from — since operators may read its fields.
            let replacement = cluster
                .operator
                .anonymize(&entities[cluster.winner], &data)
                .await?;

            // Stage a redaction event for every member, so each entity's
            // provenance reflects that this operator hid it.
            for &i in &cluster.members {
                let mut redaction =
                    Redaction::new(cluster.operator.id(), cluster.matched_by.clone())
                        .with_leak_profile(cluster.operator.leak_profile());
                if let Some(attribution) = cluster.attribution.clone() {
                    redaction = redaction.with_attribution(attribution);
                }
                let event = AuditEvent::redaction(redaction, entities[i].confidence);
                pending.push((i, event));
            }
            redactions.push(location, replacement);
        }
        // The write is the point of no return: only once it lands do the audit
        // events become true, so record them here rather than in the loop.
        target.write_at(redactions).await?;
        for (i, event) in pending {
            entities[i].audit.record(event);
        }
        Ok(())
    }
}

/// One overlap cluster resolved to the operator that will hide it: the covered
/// entity indices (in cluster order), the index of the entity that *won* the
/// operator selection, the live configured operator (cloned from the rules), and
/// the pick's provenance. The shared output of [`Anonymizer::pick`] and
/// [`Anonymizer::redact`].
struct ResolvedCluster<M: Modality> {
    /// Every covered entity's index, in cluster order (not winner-first).
    members: Vec<usize>,
    /// The member the operator was resolved from — the safest span in the
    /// cluster. An operator that reads entity fields (`Replace`, `Encrypt`, …)
    /// must run against *this* entity, not `members[0]`.
    winner: usize,
    operator: SharedOperator<M>,
    matched_by: RuleMatch,
    attribution: Option<Attribution>,
}

impl<M: Modality> Default for Anonymizer<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// Group `candidates` (indices into `entities`) that redact as one span, by
/// single-linkage clustering: two entities join when they overlap *and* their
/// locations [coalesce] into one span.
///
/// Each group is a `Vec` of indices into `entities`. Disjoint entities each
/// form a singleton; a chain of pairwise links (A–B, B–C) lands in one
/// group even if A and C don't touch, and an entity bridging two existing
/// groups merges them. Two entities that overlap but can't coalesce (the
/// same byte range on different pages, say) stay in separate groups, so
/// every group's [`union`][coalesce] is well-defined — no member is ever
/// dropped when the span is computed.
///
/// Only the `candidates` participate: the caller passes the *live* entities, so
/// a suppressed entity never bridges two live ones into a cluster that then
/// fails to coalesce once it is dropped.
///
/// [coalesce]: ModalityLocation::union
fn cluster_overlaps<M: Modality>(entities: &[Entity<M>], candidates: &[usize]) -> Vec<Vec<usize>> {
    // Two entities link only if they overlap and coalesce into one span, so
    // every group folds to a single union with no member lost.
    let links = |a: &M::Location, b: &M::Location| a.overlaps(b) && a.union(b).is_some();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &i in candidates {
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
    use elide_core::entity::audit::{AuditEvent, AuditKind, Manual, ManualIntent, RuleMatch};
    use elide_core::modality::text::{Text, TextDoc};
    use elide_core::primitive::Confidence;
    use elide_operator::operators::{Erase, Mask, Replace};

    use super::*;

    fn entity(label: &str, start: usize, end: usize) -> Entity<Text> {
        Entity::fixture_conf(label, (start, end), Confidence::new(0.9).unwrap())
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
            .map(|e| match e.audit.selection() {
                Some(s) => s.operator.name.to_string(),
                None => panic!("every entity records a Selection pick"),
            })
            .collect();
        assert_eq!(names, ["erase", "replace"]);
        assert!(
            entities.iter().all(|e| !e.is_redacted()),
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
                for_auditor[0].audit.selection(),
                Some(s) if matches!(s.matched_by, RuleMatch::Predicate)
            ),
            "auditor matches the scope predicate",
        );
        assert!(
            matches!(
                for_support[0].audit.selection(),
                Some(s) if matches!(s.matched_by, RuleMatch::Fallback)
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
            match entity.audit.selection() {
                Some(s) => assert_eq!(s.operator.name, "erase", "safest operator wins"),
                None => panic!("every cluster member records the pick"),
            }
        }
    }

    /// `redact` applies each cluster's re-resolved live operator and records the
    /// redaction on every entity it covers.
    #[tokio::test]
    async fn redact_applies_the_picks() {
        let mut target = TextDoc::new("alice and bob");
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        anonymizer
            .redact(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();
        // Each covered entity recorded the redaction.
        assert!(
            entities.iter().all(Entity::is_redacted),
            "each covered entity records its redaction",
        );
    }

    /// `anonymize` does both halves: every entity records a pick *and* a
    /// redaction, and the audit chain still verifies.
    #[tokio::test]
    async fn anonymize_picks_then_redacts() {
        let mut target = TextDoc::new("alice and bob");
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        anonymizer
            .anonymize(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();

        for entity in &entities {
            assert!(entity.audit.selection().is_some(), "records the pick");
            assert!(entity.is_redacted(), "records the redaction");
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
        let mut target = TextDoc::new("alice and bob");
        let live = entity("NAME", 0, 5);
        let mut suppressed = entity("NAME", 10, 13);
        let suppressed_id = suppressed.id;
        let live_id = live.id;
        // The reviewer marks the second detection as a false positive.
        suppressed.suppress(AuditEvent::manual(
            "manual",
            suppressed.confidence,
            Manual::new(ManualIntent::Suppress, suppressed.location.clone())
                .with_attribution(Attribution::freeform("false positive")),
        ));
        let mut entities = vec![live, suppressed];

        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        anonymizer
            .anonymize(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();

        let of = |id| entities.iter().find(|e| e.id == id).unwrap();
        assert!(
            of(live_id).audit.selection().is_some(),
            "the live entity is picked"
        );
        assert!(of(live_id).is_redacted(), "the live entity is redacted");
        assert!(
            of(suppressed_id).audit.selection().is_none(),
            "no pick for the suppressed entity",
        );
        assert!(
            !of(suppressed_id).is_redacted(),
            "the suppressed entity is left alone",
        );
        // The suppression itself is on the trail, with its attribution.
        let has_manual = of(suppressed_id).audit.events().iter().any(|e| {
            matches!(&e.kind, AuditKind::Manual(m)
                if m.attribution == Some(Attribution::freeform("false positive").into()))
        });
        assert!(
            has_manual,
            "the suppression is audited with its attribution"
        );
    }

    /// In an overlap cluster the operator runs against the *winning* entity —
    /// the safest, widest span — not `members[0]`. Two overlapping entities with
    /// different labels both resolve to `Replace`; the wider span wins, and its
    /// label is what the `{label}` template renders.
    #[tokio::test]
    async fn redact_runs_the_operator_against_the_cluster_winner() {
        let mut target = TextDoc::new("alice@example.com");
        // NAME [0,5) and EMAIL [0,17) overlap. Both -> Replace (leak profiles
        // tie), so the wider span (EMAIL) wins the tiebreak.
        let mut entities = vec![entity("NAME", 0, 5), entity("EMAIL", 0, 17)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Replace::new("[{label}]")));

        anonymizer
            .redact(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();

        // The whole span [0,17) is replaced by the winner's `{label}`. Had the
        // operator read `members[0]` (NAME) instead, the output would be
        // "[NAME]".
        assert_eq!(
            target.text(),
            "[EMAIL]",
            "the operator read the winning (wider) entity, not members[0]",
        );
    }

    /// A suppressed entity overlapping a live one contributes no operator and
    /// does not extend the redacted span, but it cannot spare the shared bytes:
    /// those fall inside the live member's own span and are still rewritten. This
    /// pins the behavior the `resolve_clusters` comment now claims.
    #[tokio::test]
    async fn a_suppressed_overlapping_entity_contributes_nothing_but_shares_bytes() {
        let mut target = TextDoc::new("alice@example.com");
        let live = entity("EMAIL", 0, 17);
        let mut suppressed = entity("NAME", 0, 5); // overlaps the live span
        let live_id = live.id;
        let suppressed_id = suppressed.id;
        suppressed.suppress(AuditEvent::manual_suppress(
            suppressed.location.clone(),
            suppressed.confidence,
        ));
        let mut entities = vec![live, suppressed];

        // The suppressed NAME would win the tiebreak on span if it counted; if it
        // leaked in, the label would be NAME. It must not.
        let anonymizer = Anonymizer::new().with(Rule::fallback(Replace::new("[{label}]")));
        anonymizer
            .redact(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();

        let of = |id| entities.iter().find(|e| e.id == id).unwrap();
        assert!(of(live_id).is_redacted(), "the live entity is redacted");
        assert!(
            !of(suppressed_id).is_redacted(),
            "the suppressed entity records no redaction",
        );
        // The redacted span is exactly the live entity's [0,17) — the suppressed
        // one did not extend it — and the operator read the live entity, so the
        // whole string becomes the live label. A leaked NAME would read "[NAME]".
        assert_eq!(
            target.text(),
            "[EMAIL]",
            "suppressed entity contributed no operator or span",
        );
    }

    /// A suppressed entity that *bridges* two non-coalescible live spans must not
    /// merge them: it is excluded from clustering, so the two live spans stay in
    /// separate clusters and each redacts on its own — no panic at the union.
    #[tokio::test]
    async fn a_suppressed_bridge_does_not_merge_live_clusters() {
        // Live [0,5) and [10,15); suppressed [4,11) overlaps both but bridges
        // spans that do not coalesce with each other.
        let mut target = TextDoc::new("0123456789abcdef");
        let live_a = entity("A", 0, 5);
        let live_b = entity("B", 10, 15);
        let mut bridge = entity("BRIDGE", 4, 11);
        let (a_id, b_id, bridge_id) = (live_a.id, live_b.id, bridge.id);
        bridge.suppress(AuditEvent::manual_suppress(
            bridge.location.clone(),
            bridge.confidence,
        ));
        let mut entities = vec![live_a, bridge, live_b];

        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));
        anonymizer
            .redact(&mut target, &mut entities, &Scope::default())
            .await
            .unwrap();

        let of = |id| entities.iter().find(|e| e.id == id).unwrap();
        // Two separate redactions ([0,5) and [10,15)); the bridge is untouched.
        assert!(of(a_id).is_redacted(), "live A redacts on its own");
        assert!(of(b_id).is_redacted(), "live B redacts on its own");
        assert!(
            !of(bridge_id).is_redacted(),
            "the suppressed bridge is left alone"
        );
        // Each live span erased separately: [0,5) and [10,15) removed, leaving
        // the middle gap [5,10) and the tail [15,16).
        assert_eq!(target.text(), "56789f");
    }

    /// A failed write records no redaction events: the audit trail is the
    /// compliance artifact, so it must never claim an operator hid an entity when
    /// the document was left unchanged.
    #[tokio::test]
    async fn a_failed_write_records_no_redaction_events() {
        let mut target = TextDoc::failing("alice and bob");
        let mut entities = vec![entity("NAME", 0, 5), entity("NAME", 10, 13)];
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        let result = anonymizer
            .redact(&mut target, &mut entities, &Scope::default())
            .await;

        assert!(result.is_err(), "the write failed");
        assert!(
            entities.iter().all(|e| !e.is_redacted()),
            "no entity records a redaction the write never applied",
        );
    }
}
