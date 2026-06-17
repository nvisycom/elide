//! The [`Redactions`] batch — the output of anonymizing entities.

use veil_core::modality::Modality;

/// A batch of `(location, replacement)` pairs ready to be applied to a
/// document by a codec.
///
/// The output of [`Anonymizer::anonymize`](crate::Anonymizer::anonymize):
/// each entry says *where* (a [`Location`](Modality::Location)) and
/// *what* (a [`Replacement`](Modality::Replacement)) to write. The
/// anonymizer only computes these; applying them back into the source is
/// the codec's job — which keeps redaction free of format knowledge.
#[derive(Debug, Clone)]
pub struct Redactions<M: Modality> {
    items: Vec<(M::Location, M::Replacement)>,
}

impl<M: Modality> Redactions<M> {
    /// An empty batch.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add a `(location, replacement)` pair.
    pub fn push(&mut self, location: M::Location, replacement: M::Replacement) {
        self.items.push((location, replacement));
    }

    /// The `(location, replacement)` pairs.
    pub fn items(&self) -> &[(M::Location, M::Replacement)] {
        &self.items
    }

    /// Consume the batch into its pairs.
    pub fn into_items(self) -> Vec<(M::Location, M::Replacement)> {
        self.items
    }

    /// The number of redactions.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<M: Modality> Default for Redactions<M> {
    fn default() -> Self {
        Self::new()
    }
}
