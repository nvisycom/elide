//! The [`Anonymizer`] — the "hide" engine.
//!
//! The redaction counterpart to [`Analyzer`]: a label→operator map plus
//! two entry points. [`anonymize`] picks each entity's operator,
//! computes its [`Replacement`], and applies the batch back into the
//! target in one step; [`plan`] stops a step short and hands back the
//! [`Redactions`] batch for inspection or deferred application.
//!
//! [`Analyzer`]: crate::Analyzer
//! [`anonymize`]: Anonymizer::anonymize
//! [`plan`]: Anonymizer::plan
//! [`Replacement`]: veil_core::modality::Modality::Replacement

mod dyn_operator;
pub mod operators;
mod registry;

use veil_core::Error;
use veil_core::entity::{Entity, LabelRef};
use veil_core::modality::{DataReader, DataWriter, Modality};
use veil_core::redaction::{Operator, Redactions};

use self::registry::OperatorRegistry;

/// The hide engine: selects an operator per entity label and computes
/// its replacement.
///
/// Generic over the [`Modality`] `M`. Operators are mapped to labels
/// with [`with_operator`]; an optional [`with_fallback`] covers unmapped
/// labels. [`anonymize`] resolves and runs the operators, applying the
/// replacements back into the target.
///
/// ```ignore
/// Anonymizer::new()
///     .with_operator(LabelRef::new("PHONE_NUMBER"), Mask::stars())
///     .with_operator(LabelRef::new("EMAIL_ADDRESS"), Replace::default())
///     .with_fallback(Redact)
///     .anonymize(&mut document, &entities)
///     .await?;
/// ```
///
/// [`with_operator`]: Anonymizer::with_operator
/// [`with_fallback`]: Anonymizer::with_fallback
/// [`anonymize`]: Anonymizer::anonymize
pub struct Anonymizer<M: Modality> {
    operators: OperatorRegistry<M>,
}

impl<M: Modality> Anonymizer<M> {
    /// An anonymizer with no operators.
    pub fn new() -> Self {
        Self {
            operators: OperatorRegistry::new(),
        }
    }

    /// Map an operator to a label.
    #[must_use]
    pub fn with_operator<O: Operator<M> + 'static>(mut self, label: LabelRef, operator: O) -> Self {
        self.operators.insert(label, operator);
        self
    }

    /// Set the fallback operator for labels with no specific mapping.
    #[must_use]
    pub fn with_fallback<O: Operator<M> + 'static>(mut self, operator: O) -> Self {
        self.operators.set_fallback(operator);
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
        entities: &[Entity<M>],
        reader: &impl DataReader<M>,
    ) -> Result<Redactions<M>, Error> {
        let mut redactions = Redactions::new();
        for entity in entities {
            let Some(operator) = self.operators.resolve(&entity.label) else {
                tracing::debug!(
                    modality = M::NAME,
                    label = entity.label.as_str(),
                    "no operator for label; skipping",
                );
                continue;
            };
            let Some(data) = reader.read_at(&entity.location).await? else {
                tracing::debug!(
                    modality = M::NAME,
                    label = entity.label.as_str(),
                    "location read no data; skipping",
                );
                continue;
            };
            let replacement = operator.anonymize_boxed(entity, &data).await?;
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
    pub async fn anonymize<T>(&self, target: &mut T, entities: &[Entity<M>]) -> Result<(), Error>
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
