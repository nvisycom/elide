//! [`MarkupSink`]: the growing item stream, keeping its engine-space offset in
//! step so a chunk's offset can never drift from the stream it indexes.

use elide_codec::extract::{ExtractedItem, ItemFlag, Passage};
use elide_core::modality::ResolvedHint;
use elide_core::modality::text::Text;

use super::address::{MarkupItem, MarkupSpan};

/// The growing item stream: the extracted items and the running engine-space
/// offset (the cumulative length of their values). [`push`](Self::push) keeps
/// the two in step, every item advances the offset by its own length, so the
/// offset a chunk carries can never drift from the stream it indexes.
pub(crate) struct MarkupSink {
    items: Vec<MarkupItem>,
    offset: usize,
}

impl MarkupSink {
    pub(crate) fn new() -> Self {
        Self {
            items: Vec::new(),
            offset: 0,
        }
    }

    /// The current engine-space offset (where the next item's value begins).
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Append an item for the value read at `address`, joining the passage
    /// before it as `passage` says, and advance the offset by the value's
    /// length. Returns the new item's index.
    pub(crate) fn push(&mut self, address: MarkupSpan, passage: Passage) -> usize {
        let value = address.read.clone();
        self.offset += value.len();
        let index = self.items.len();
        self.items.push(ExtractedItem {
            value,
            address,
            hints: Vec::new(),
            passage,
            flag: None,
        });
        index
    }

    /// Mark the item at `index` as the hidden content `flag` says it is.
    pub(crate) fn flag(&mut self, index: usize, flag: ItemFlag) {
        self.items[index].flag = Some(flag);
    }

    /// Append `hint` to the hints of the item at `index`.
    pub(crate) fn hint(&mut self, index: usize, hint: ResolvedHint<Text>) {
        self.items[index].hints.push(hint);
    }

    /// The finished item stream.
    pub(crate) fn into_items(self) -> Vec<MarkupItem> {
        self.items
    }
}
