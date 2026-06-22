//! The [`Deanonymizer`] — the "recover" engine.
//!
//! The reverse of [`Anonymizer`]: for each entity it resolves a
//! [`ReversibleOperator`] (e.g. [`Encrypt`]), reads the current
//! replacement text the document holds, recovers the original, and writes
//! it back. Only [`TextBacked`] modalities are supported — recovery
//! reconstructs a [`TextReplacement`] from the stored text, which is
//! well-defined only where the data *is* the text.
//!
//! [`Anonymizer`]: crate::Anonymizer
//! [`ReversibleOperator`]: elide_core::redaction::ReversibleOperator
//! [`Encrypt`]: crate::redaction::operators::Encrypt
//! [`TextBacked`]: elide_core::modality::TextBacked
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

mod dyn_reversible;
mod registry;

use elide_core::Result;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::TextReplacement;
use elide_core::modality::{DataReader, DataWriter, Modality, TextBacked};
use elide_core::redaction::{Redactions, ReversibleOperator};

use self::registry::ReversibleRegistry;

/// The recover engine: selects a reversible operator per entity and
/// recovers the original value it replaced.
///
/// Generic over a [`TextBacked`] modality `M`. Selection is an ordered list
/// of rules tried top to bottom, first match winning: bind an operator to a
/// label with [`with_label`], or as a catch-all with [`with_fallback`].
/// [`deanonymize`] resolves and runs the operators, writing the recovered
/// originals back into the target.
///
/// Pairs with [`Anonymizer`]: encrypt under a label on the way in, decrypt
/// under the same label on the way out.
///
/// [`with_label`]: Deanonymizer::with_label
/// [`with_fallback`]: Deanonymizer::with_fallback
/// [`deanonymize`]: Deanonymizer::deanonymize
/// [`Anonymizer`]: crate::Anonymizer
pub struct Deanonymizer<M: Modality> {
    operators: ReversibleRegistry<M>,
}

impl<M: TextBacked> Deanonymizer<M> {
    /// A deanonymizer with no rules.
    pub fn new() -> Self {
        Self {
            operators: ReversibleRegistry::new(),
        }
    }

    /// Append a rule binding `operator` to an exact label.
    #[must_use]
    pub fn with_label<O: ReversibleOperator<M> + 'static>(
        mut self,
        label: LabelRef,
        operator: O,
    ) -> Self {
        self.operators.push_label(label, operator);
        self
    }

    /// Append a catch-all rule: `operator` runs for every entity not
    /// matched by an earlier rule.
    #[must_use]
    pub fn with_fallback<O: ReversibleOperator<M> + 'static>(mut self, operator: O) -> Self {
        self.operators.push_fallback(operator);
        self
    }

    /// Plan the recovery for every entity, reading each one's current value
    /// from `reader`, without applying anything.
    ///
    /// For each entity: resolve its reversible operator, read the current
    /// (replaced) value, reconstruct the [`TextReplacement`] it represents,
    /// and recover the original. Entities with no operator, no readable
    /// data, or an unrecoverable value (wrong key, not produced by this
    /// operator) are skipped. Returns the [`Redactions`] batch of recovered
    /// originals.
    pub async fn plan(
        &self,
        entities: &[Entity<M>],
        reader: &impl DataReader<M>,
    ) -> Result<Redactions<M>> {
        let mut redactions = Redactions::new();
        for entity in entities {
            let Some(operator) = self.operators.resolve(entity) else {
                continue;
            };
            let Some(data) = reader.read_at(&entity.location).await? else {
                continue;
            };
            // The document holds the replacement text as data; lift it back
            // to a replacement so the operator can reverse it. For a
            // `TextBacked` modality `M::Data` is `TextData`.
            let current = TextReplacement::substituted(data.as_str());
            let Some(original) = operator.deanonymize_boxed(entity, &current).await? else {
                tracing::debug!(
                    modality = M::NAME,
                    label = entity.label.as_str(),
                    "value not recoverable; skipping",
                );
                continue;
            };
            // Write the recovered original back as a substitution.
            redactions.push(
                entity.location.clone(),
                TextReplacement::substituted(original.as_str()),
            );
        }
        Ok(redactions)
    }

    /// Recover every entity by writing its operator's recovered original
    /// back into `target`.
    ///
    /// The complete recovery step: [`plan`]s each entity's original
    /// (reading its current value from `target`), then hands the batch to
    /// [`DataWriter::write_at`]. `target` is both reader and writer.
    ///
    /// [`plan`]: Self::plan
    pub async fn deanonymize<T>(&self, target: &mut T, entities: &[Entity<M>]) -> Result<()>
    where
        T: DataReader<M> + DataWriter<M>,
    {
        let redactions = self.plan(entities, target).await?;
        target.write_at(redactions).await
    }
}

impl<M: TextBacked> Default for Deanonymizer<M> {
    fn default() -> Self {
        Self::new()
    }
}
