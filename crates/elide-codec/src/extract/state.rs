//! The mutable decoded item stream (`SpliceState<A>`) an
//! [`ExtractStream`](super::ExtractStream) owns, with its offset index.

use std::ops::Range;

use elide_core::modality::text::TextReplacement;
use elide_core::{Error, Result};

use super::{ExtractedItem, ItemEdit, Passage};
use crate::string::{ContiguousSpan, RedactRange};

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

    /// The text `range` (a range of the concatenated stream) covers.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if `range` runs past the stream or splits a
    /// character.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn text(&self, range: Range<usize>) -> Result<String> {
        let edits = self.covering(range.clone()).ok_or_else(|| {
            Error::malformed(format!(
                "range {}..{} is outside the text",
                range.start, range.end
            ))
        })?;
        edits
            .into_iter()
            .map(|edit| self.items[edit.item].value.get(edit.local))
            .collect::<Option<String>>()
            .ok_or_else(|| {
                Error::malformed(format!(
                    "range {}..{} splits a UTF-8 character",
                    range.start, range.end
                ))
            })
    }

    /// The one span of the concatenated stream that `edits` cover together,
    /// as [`ContiguousSpan`] joins them.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if there are no edits, one is outside its item, or
    /// anything but whitespace separates two of them.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn contiguous_span(&self, edits: &[ItemEdit]) -> Result<Range<usize>> {
        let mut pieces = Vec::with_capacity(edits.len());
        for ItemEdit { item, local } in edits {
            let (Some(&start), Some(&end)) =
                (self.item_starts.get(*item), self.item_starts.get(item + 1))
            else {
                return Err(Error::malformed(format!(
                    "item {item} is outside the stream"
                )));
            };
            if local.start > local.end || start + local.end > end {
                return Err(Error::malformed(format!(
                    "range {}..{} is outside item {item}",
                    local.start, local.end
                )));
            }
            pieces.push(start + local.start..start + local.end);
        }
        // Join within the stretch the pieces span, not the whole stream.
        let (Some(start), Some(end)) = (
            pieces.iter().map(|piece| piece.start).min(),
            pieces.iter().map(|piece| piece.end).max(),
        ) else {
            return Err(Error::malformed("the mark covers no text"));
        };
        let hull = start..end;
        let window = self.text(hull.clone())?;
        let span = window.contiguous_span(
            pieces
                .into_iter()
                .map(|piece| piece.start - hull.start..piece.end - hull.start),
        )?;
        Ok(hull.start + span.start..hull.start + span.end)
    }

    /// Write `replacement` over `span`, a range of the concatenated stream.
    ///
    /// A cover marks each item's part of the span in place, so every item
    /// keeps its extent. Any other replacement's text takes the place of the
    /// first item's part, and the rest are cut. Either way each item keeps its
    /// own markup, and separators are never edited.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if `span` runs past the stream or splits a
    /// character.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn redact(
        &mut self,
        span: Range<usize>,
        replacement: &TextReplacement,
    ) -> Result<()> {
        let edits: Vec<ItemEdit> = self
            .covering(span.clone())
            .ok_or_else(|| {
                Error::malformed(format!(
                    "range {}..{} is outside the text",
                    span.start, span.end
                ))
            })?
            .into_iter()
            .filter(|edit| self.items[edit.item].passage != Passage::Separator)
            .collect();
        let Some(first) = edits.first().map(|edit| edit.item) else {
            return Ok(());
        };
        let covered = matches!(replacement, TextReplacement::Covered { .. });
        let written = if covered {
            String::new()
        } else {
            let original = edits
                .iter()
                .filter_map(|edit| self.items[edit.item].value.get(edit.local.clone()))
                .collect::<String>();
            replacement.written(&original).into_owned()
        };
        // Right to left, so an edit never shifts the ones still to come.
        for ItemEdit { item, local } in edits.into_iter().rev() {
            let before_len = self.items[item].value.len();
            let value = &mut self.items[item].value;
            if covered {
                value.redact_with(replacement, local)?;
            } else {
                let text = if item == first { written.as_str() } else { "" };
                value.redact_range(text, local)?;
            }
            let delta = self.items[item].value.len() as isize - before_len as isize;
            self.shift_starts_after(item, delta);
        }
        Ok(())
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
