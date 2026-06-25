//! [`Redactions`] batch: what an anonymizer hands a codec to apply.

use std::{slice, vec};

use crate::modality::{Modality, ModalityLocation};

/// Batch of `(location, replacement)` pairs for a codec to apply.
///
/// The output of anonymizing a set of entities: each entry says *where*
/// (a [`Location`]) and *what* (a [`Replacement`]) to write. The
/// anonymizer only computes these; applying them back into the source is
/// the codec's job, which keeps redaction free of format knowledge.
///
/// Entries accumulate in [`push`] order. A codec that rewrites a medium
/// in a single forward pass wants them in document order instead;
/// [`sort_by_position`] reorders the batch in place by
/// [`ModalityLocation::position_cmp`]. Iterate the pairs with [`iter`]
/// or by value via [`IntoIterator`].
///
/// [`Location`]: Modality::Location
/// [`Replacement`]: Modality::Replacement
/// [`push`]: Redactions::push
/// [`sort_by_position`]: Redactions::sort_by_position
/// [`ModalityLocation::position_cmp`]: crate::modality::ModalityLocation::position_cmp
/// [`iter`]: Redactions::iter
#[derive(Debug)]
pub struct Redactions<M: Modality> {
    items: Vec<(M::Location, M::Replacement)>,
}

// Manual `Clone`: `derive` would add a spurious `M: Clone` bound, but `M`
// is a zero-size marker. The contents clone via the location/replacement
// bounds the modality already guarantees.
impl<M: Modality> Clone for Redactions<M> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
        }
    }
}

impl<M: Modality> Redactions<M> {
    /// Empty batch.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add a `(location, replacement)` pair.
    pub fn push(&mut self, location: M::Location, replacement: M::Replacement) {
        self.items.push((location, replacement));
    }

    /// Sort the batch in place by position in the medium (document order).
    ///
    /// Orders by [`ModalityLocation::position_cmp`] so a codec can apply
    /// redactions in a single deterministic pass. The sort is stable, so two
    /// redactions at the same position keep their insertion order.
    ///
    /// [`ModalityLocation::position_cmp`]: crate::modality::ModalityLocation::position_cmp
    pub fn sort_by_position(&mut self) {
        self.items.sort_by(|(a, _), (b, _)| a.position_cmp(b));
    }

    /// Iterate the `(location, replacement)` pairs by reference, in the
    /// batch's current order.
    ///
    /// Returns a [`slice::Iter`], a double-ended iterator, so callers can
    /// walk the batch in reverse with `.rev()` (a writer applies a
    /// position-sorted batch back-to-front so length changes don't shift
    /// later locations).
    pub fn iter(&self) -> slice::Iter<'_, (M::Location, M::Replacement)> {
        self.items.iter()
    }

    /// Number of redactions.
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
    type IntoIter = vec::IntoIter<Self::Item>;
    type Item = (M::Location, M::Replacement);

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a, M: Modality> IntoIterator for &'a Redactions<M> {
    type IntoIter = slice::Iter<'a, (M::Location, M::Replacement)>;
    type Item = &'a (M::Location, M::Replacement);

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}
