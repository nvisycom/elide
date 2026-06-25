//! The [`Deanonymizer`] — the "recover" engine.
//!
//! The reverse of [`Anonymizer`]: per entity it resolves a
//! [`ReversibleOperator`] (e.g. [`AesEncrypt`]), reads the replacement text the
//! document holds, recovers the original, and writes it back. Supported for
//! modalities whose data and recoverable replacement are both text ([`Text`]
//! and `Tabular`), where the stored value can be lifted back to a
//! [`TextReplacement`] for the operator to reverse.
//!
//! [`Anonymizer`]: crate::Anonymizer
//! [`ReversibleOperator`]: elide_core::operator::ReversibleOperator
//! [`AesEncrypt`]: crate::operators::AesEncrypt
//! [`Text`]: elide_core::modality::text::Text
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

mod dyn_reversible;
mod registry;

use elide_core::Result;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::modality::{DataReader, DataWriter, Modality, TextRecognizable};
use elide_core::operator::{Redactions, ReversibleOperator};

use self::registry::ReversibleRegistry;

/// The recover engine: picks a reversible operator per entity and restores
/// the original value.
///
/// Selection is an ordered list of rules tried top to bottom, first match
/// winning: bind an operator to a label with [`with_label`], or as a
/// catch-all with [`with_fallback`]. [`deanonymize`] resolves and runs the
/// operators, writing the recovered originals back into the target.
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

impl<M: Modality> Deanonymizer<M> {
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
}

/// Recovery for a text-backed modality: the stored replacement is text, so
/// it lifts to a [`TextReplacement`] (and into `M::Replacement`) the operator
/// can reverse. Implemented for [`Text`] and `Tabular`.
///
/// [`Text`]: elide_core::modality::text::Text
impl<M> Deanonymizer<M>
where
    M: TextRecognizable<Data = TextData>,
    M::Replacement: From<TextReplacement>,
{
    /// Plan the recovery for every entity, reading each one's current value
    /// from `reader`, without applying anything.
    ///
    /// For each entity: resolve its reversible operator, read the current
    /// (replaced) value, lift it back to the modality's replacement, and
    /// recover the original. Entities with no operator, no readable data, or
    /// an unrecoverable value (wrong key, not produced by this operator) are
    /// skipped.
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
            // to the modality's replacement so the operator can reverse it.
            let current = M::Replacement::from(TextReplacement::substituted(data.as_str()));
            let Some(original) = operator.deanonymize_boxed(entity, &current).await? else {
                tracing::debug!(
                    modality = M::NAME,
                    label = entity.label.as_str(),
                    "value not recoverable; skipping",
                );
                continue;
            };
            redactions.push(
                entity.location.clone(),
                M::Replacement::from(TextReplacement::substituted(original.as_str())),
            );
        }
        Ok(redactions)
    }

    /// Recover every entity by writing its operator's recovered original
    /// back into `target`.
    ///
    /// The complete recovery step: [`plan`]s each entity's original (reading
    /// its current value from `target`), then hands the batch to
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

impl<M: Modality> Default for Deanonymizer<M> {
    fn default() -> Self {
        Self::new()
    }
}
