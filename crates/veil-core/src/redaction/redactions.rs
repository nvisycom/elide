//! The [`Redactions`] batch — what an anonymizer hands a codec to apply.

use crate::modality::{Modality, ModalityLocation};

/// A batch of `(location, replacement)` pairs ready to be applied to a
/// document by a codec.
///
/// The output of anonymizing a set of entities: each entry says *where*
/// (a [`Location`](Modality::Location)) and *what* (a
/// [`Replacement`](Modality::Replacement)) to write. The anonymizer only
/// computes these; applying them back into the source is the codec's job
/// — which keeps redaction free of format knowledge.
///
/// Entries accumulate in [`push`](Redactions::push) order. A codec that
/// rewrites a medium in a single forward pass wants them in document
/// order instead; [`sort_by_position`](Redactions::sort_by_position)
/// reorders the batch in place by
/// [`ModalityLocation::position_cmp`](crate::modality::ModalityLocation::position_cmp).
/// Iterate the pairs with [`iter`](Redactions::iter) or by value via
/// [`IntoIterator`].
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

    /// Sort the batch in place by position in the medium (document
    /// order).
    ///
    /// Orders by
    /// [`ModalityLocation::position_cmp`](crate::modality::ModalityLocation::position_cmp)
    /// so a codec can apply redactions in a single deterministic pass.
    /// The sort is stable, so two redactions at the same position keep
    /// their insertion order.
    pub fn sort_by_position(&mut self) {
        self.items.sort_by(|(a, _), (b, _)| a.position_cmp(b));
    }

    /// Iterate the `(location, replacement)` pairs by reference, in the
    /// batch's current order.
    pub fn iter(&self) -> impl Iterator<Item = &(M::Location, M::Replacement)> {
        self.items.iter()
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

impl<M: Modality> IntoIterator for Redactions<M> {
    type IntoIter = std::vec::IntoIter<Self::Item>;
    type Item = (M::Location, M::Replacement);

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a, M: Modality> IntoIterator for &'a Redactions<M> {
    type IntoIter = std::slice::Iter<'a, (M::Location, M::Replacement)>;
    type Item = &'a (M::Location, M::Replacement);

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}
