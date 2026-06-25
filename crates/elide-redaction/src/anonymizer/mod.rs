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
pub mod operators;
mod registry;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::provenance::{Attribution, Event};
use elide_core::entity::{Entity, LabelCatalog, LabelRef};
use elide_core::modality::{DataReader, DataWriter, Modality};
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
    /// are skipped, as are entities whose location reads no data. Returns
    /// the [`Redactions`] batch — inspect, serialize, or audit it, then
    /// apply it yourself, or call [`anonymize`] to plan and apply in one
    /// step.
    ///
    /// [`anonymize`]: Self::anonymize
    pub async fn plan(
        &self,
        entities: &mut [Entity<M>],
        reader: &impl DataReader<M>,
    ) -> Result<Redactions<M>> {
        let mut redactions = Redactions::new();
        for entity in entities {
            let Some(resolved) = self.operators.resolve(entity) else {
                tracing::debug!(
                    modality = M::NAME,
                    label = entity.label.as_str(),
                    "no operator for label; skipping",
                );
                continue;
            };
            // Clone what we need so the registry borrow ends before we take
            // `&mut entity` to record provenance below.
            let operator = Arc::clone(resolved.operator);
            let matched_by = resolved.matched_by;
            let attribution = resolved.attribution.cloned();
            let Some(data) = reader.read_at(&entity.location).await? else {
                tracing::debug!(
                    modality = M::NAME,
                    label = entity.label.as_str(),
                    "location read no data; skipping",
                );
                continue;
            };
            let replacement = operator.anonymize_boxed(entity, &data).await?;

            // Record why and how this entity was redacted: the matched rule
            // (automatic "why"), the operator identity + leak profile, and the
            // rule's author-supplied attribution (the policy "why").
            let event = Event::redaction(
                operator.id(),
                operator.leak_profile(),
                entity.confidence,
                matched_by,
                attribution,
            );
            entity.provenance.record(event);

            redactions.push(entity.location.clone(), replacement);
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
