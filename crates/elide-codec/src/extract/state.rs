//! The mutable decoded item stream ([`SpliceState<A>`]) and its `Clone`-to-share
//! handle ([`SharedSplice<A>`]), with the lock hidden behind methods.

use std::sync::{Arc, Mutex};

use super::ExtractedItem;

/// The mutable decoded item stream of a structured document, shared between an
/// [`ExtractStream`](super::ExtractStream) and the format's
/// [`Recombine`](crate::Recombine).
///
/// `item_starts` is a cumulative-offset index over the items:
/// `item_starts[i]` is the byte position of item `i` in the concatenated
/// item-value stream, and `item_starts[items.len()]` is the total-length
/// sentinel. Maintained on every redaction so random-access reads run in
/// `O(log N)`. Offsets are over the redactable-item sequence in document
/// order, not raw source bytes.
#[derive(Debug)]
pub struct SpliceState<A> {
    /// The redactable item stream, in document order.
    pub items: Vec<ExtractedItem<A>>,
    /// The cumulative byte-offset index over `items`.
    pub item_starts: Vec<usize>,
}

impl<A> SpliceState<A> {
    /// Build the state (and its offset index) from a decoded item stream.
    pub fn new(items: Vec<ExtractedItem<A>>) -> Self {
        let item_starts = compute_item_starts(&items);
        Self { items, item_starts }
    }

    /// The item whose value contains `byte_offset` in the concatenated stream.
    pub(super) fn item_for(&self, byte_offset: usize) -> Option<usize> {
        // The last item whose start is `<= byte_offset`. An empty item shares its
        // start with the next, so several starts tie at one offset; only the last
        // of them can hold a non-empty range, and `binary_search` would pick an
        // arbitrary tie (possibly the empty item, skipping the redaction).
        let i = self
            .item_starts
            .partition_point(|&start| start <= byte_offset)
            .checked_sub(1)?;
        (i < self.items.len()).then_some(i)
    }

    /// Shift every offset after item `i` by `delta` (an edit's length change).
    pub(super) fn shift_starts_after(&mut self, i: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for s in &mut self.item_starts[i + 1..] {
            *s = s.saturating_add_signed(delta);
        }
    }
}

/// A `Clone`-to-share handle to a [`SpliceState`], so an
/// [`ExtractStream`](super::ExtractStream) and a [`Recombine`](crate::Recombine)
/// share (and both see) the redacted items. The lock is held only inside these
/// methods — a `Recombine` reads the redacted items through
/// [`with_items`](Self::with_items) rather than locking directly.
pub struct SharedSplice<A>(Arc<Mutex<SpliceState<A>>>);

impl<A> Clone for SharedSplice<A> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<A: std::fmt::Debug> std::fmt::Debug for SharedSplice<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedSplice")
            .field(&*self.0.lock().unwrap())
            .finish()
    }
}

impl<A> SharedSplice<A> {
    /// Build a shared state from a decoded item stream.
    pub fn new(items: Vec<ExtractedItem<A>>) -> Self {
        Self(Arc::new(Mutex::new(SpliceState::new(items))))
    }

    /// Read the (possibly redacted) items under the lock — how a
    /// [`Recombine`](crate::Recombine) re-serialises them into native bytes.
    pub fn with_items<R>(&self, f: impl FnOnce(&[ExtractedItem<A>]) -> R) -> R {
        f(&self.0.lock().unwrap().items)
    }

    /// The state under the lock, for the co-located
    /// [`ExtractStream`](super::ExtractStream)'s read/redact paths.
    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, SpliceState<A>> {
        self.0.lock().unwrap()
    }
}

/// Cumulative byte-offset table over the items: `[0, len(item[0]),
/// len(item[0]) + len(item[1]), …, total]`.
fn compute_item_starts<A>(items: &[ExtractedItem<A>]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(items.len() + 1);
    let mut offset = 0usize;
    for item in items {
        starts.push(offset);
        offset += item.value.len();
    }
    starts.push(offset);
    starts
}
