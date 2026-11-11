//! The mutable decoded item stream (`SpliceState<A>`) an
//! [`ExtractStream`](super::ExtractStream) owns, with its offset index.

use std::ops::Range;

use super::{ExtractedItem, ItemEdit, Passage};

/// The mutable decoded item stream of a structured document, owned by its
/// [`ExtractStream`](super::ExtractStream).
///
/// `item_starts` is a cumulative-offset index over the items:
/// `item_starts[i]` is the byte position of item `i` in the concatenated
/// item-value stream, and `item_starts[items.len()]` is the total-length
/// sentinel. Maintained on every redaction so random-access reads run in
/// `O(log N)`. Offsets are over the redactable-item sequence in document
/// order, not raw source bytes.
#[derive(Debug)]
pub(super) struct SpliceState<A> {
    /// The redactable item stream, in document order.
    pub(super) items: Vec<ExtractedItem<A>>,
    /// The cumulative byte-offset index over `items`.
    pub(super) item_starts: Vec<usize>,
}

impl<A> SpliceState<A> {
    /// Build the state (and its offset index) from a decoded item stream.
    pub(super) fn new(items: Vec<ExtractedItem<A>>) -> Self {
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

    /// The item index range of each passage, in order.
    pub(super) fn passages(&self) -> Vec<Range<usize>> {
        let mut passages = Vec::new();
        let mut start = 0;
        for (i, item) in self.items.iter().enumerate().skip(1) {
            if item.passage == Passage::Start {
                passages.push(start..i);
                start = i;
            }
        }
        if start < self.items.len() {
            passages.push(start..self.items.len());
        }
        passages
    }

    /// The items `range` (a range of the concatenated stream) covers, each with
    /// the part of its value covered, in item order; `None` when `range` runs
    /// past the stream. An item `range` only touches at an edge is left out,
    /// unless `range` is empty and touches nothing else.
    pub(super) fn covering(&self, range: Range<usize>) -> Option<Vec<ItemEdit>> {
        if range.start > range.end || range.end > *self.item_starts.last()? {
            return None;
        }
        let first = self.item_for(range.start)?;
        let mut edits = Vec::new();
        for item in first..self.items.len() {
            let start = self.item_starts[item];
            let end = self.item_starts[item + 1];
            if item > first && start >= range.end {
                break;
            }
            let local = range.start.max(start) - start..range.end.min(end) - start;
            if item == first || !local.is_empty() {
                edits.push(ItemEdit { item, local });
            }
        }
        Some(edits)
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
