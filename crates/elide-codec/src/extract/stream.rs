//! [`ExtractStream<A>`]: the [`Stream`] machinery over an extracted item
//! stream.

use std::ops::Range;

use elide_core::modality::text::{SourceRef, Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::{ExtractedItem, ItemEdit, Passage, SourceAddresser, SpliceState};
use crate::string::RedactRange;
use crate::{FlaggedItem, FormatId, Stream};

/// The [`Stream`] over a structured document's extracted, redactable items.
///
/// It yields each [`Passage`] of items as one [`Chunk`], so an entity a
/// document stores across several items is read whole; reads and redacts by
/// decoded offset, across items; and lifts a chunk-local finding to source
/// coordinates via the [`SourceAddresser`]. It owns the decoded items; the
/// format's [`Native`](crate::Native) document reads the redacted ones through
/// [`items`](Self::items) to write the document out.
///
/// [`Chunk`]: elide_core::modality::Chunk
pub struct ExtractStream<A> {
    format_id: FormatId,
    state: SpliceState<A>,
    addresser: Box<dyn SourceAddresser<A>>,
}

impl<A: Send + Sync + 'static> ExtractStream<A> {
    /// Build a stream over the decoded `items`, addressed by `addresser`.
    pub fn new(
        format_id: FormatId,
        items: Vec<ExtractedItem<A>>,
        addresser: impl SourceAddresser<A> + 'static,
    ) -> Self {
        Self {
            format_id,
            state: SpliceState::new(items),
            addresser: Box::new(addresser),
        }
    }

    /// The (possibly redacted) items, in document order.
    pub fn items(&self) -> &[ExtractedItem<A>] {
        &self.state.items
    }

    /// The location of the decoded range `range`, with the exact raw source
    /// range(s) it came from, item by item, when the addresser can map them
    /// (markup, whose item value is a verbatim source slice; OOXML, via its
    /// per-block offset map). A separator has no source text of its own.
    fn source_location(&self, range: Range<usize>) -> TextLocation {
        let state = &self.state;
        let source: Vec<SourceRef> = state
            .covering(range.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|edit| {
                !edit.local.is_empty() && state.items[edit.item].passage != Passage::Separator
            })
            .flat_map(|edit| {
                self.addresser
                    .source_span(&state.items[edit.item], edit.local)
            })
            .collect();
        TextLocation::source(range, source)
    }

    fn redact_one(
        addresser: &dyn SourceAddresser<A>,
        state: &mut SpliceState<A>,
        location: &TextLocation,
        replacement: &TextReplacement,
    ) -> Result<()> {
        // Resolve to the item and its decoded-local range. A location addressed
        // by a raw `source` reference, e.g. an entity a review layer added by
        // selecting text in a container part, is reverse-resolved through the
        // addresser; otherwise the decoded `range` locates the item directly.
        // Either way the edit path below is the same.
        let edits: Vec<ItemEdit> = Self::resolve(addresser, state, location)?
            .into_iter()
            .filter(|edit| state.items[edit.item].passage != Passage::Separator)
            .collect();
        let Some(first) = edits.first().map(|edit| edit.item) else {
            return Ok(());
        };
        // The replacement takes the place of the covered text of the first item,
        // and the covered text of the rest is cut, so each item keeps its own
        // markup. Right to left, so an edit never shifts the ones still to come.
        let value = replacement.value().unwrap_or_default();
        for ItemEdit { item, local } in edits.into_iter().rev() {
            let text = if item == first { value } else { "" };
            let before_len = state.items[item].value.len();
            state.items[item].value.redact_range(text, local)?;
            let delta = state.items[item].value.len() as isize - before_len as isize;
            state.shift_starts_after(item, delta);
        }
        Ok(())
    }

    /// Resolve `location` to the [`ItemEdit`]s it targets, one per item it
    /// covers.
    ///
    /// Prefers a raw [`source`](TextLocation::source) reference when present
    /// (reverse-resolved via [`SourceAddresser::locate_source`]); falls back to
    /// the decoded [`range`](TextLocation::range).
    ///
    /// An unresolvable **`source`** reference is a caller mistake, not a no-op:
    /// the caller explicitly supplied raw byte coordinates to redact, and a bad
    /// part name, an offset past the part, or a cross-part span means those
    /// bytes were never redacted. Returning `Ok(None)` there would let a green
    /// audit stand over an unredacted document, so it is an `Err` instead. A
    /// missed decoded `range` resolves to nothing: the pipeline's own
    /// coordinate, tolerated as a no-op.
    fn resolve(
        addresser: &dyn SourceAddresser<A>,
        state: &SpliceState<A>,
        location: &TextLocation,
    ) -> Result<Vec<ItemEdit>> {
        let source = location.source_refs();
        if !source.is_empty() {
            let edits = addresser.locate_source(&state.items, source);
            if edits.is_empty() {
                // A raw source reference that resolves to no redactable
                // item (a bad part name, an offset past the part, a span
                // crossing parts) is the caller's coordinate at fault.
                let refs: Vec<String> = source
                    .iter()
                    .map(|src| match &src.part {
                        Some(part) => {
                            format!("{part}#{}..{}", src.range.start, src.range.end)
                        }
                        None => format!("{}..{}", src.range.start, src.range.end),
                    })
                    .collect();
                return Err(Error::malformed(format!(
                    "source reference resolves to no redactable item: {}",
                    refs.join(" ")
                )));
            }
            return Ok(edits);
        }
        // No source refs and no decoded range: a source-only location with an
        // empty ref set, nothing to resolve. A decoded range is the pipeline's
        // own coordinate, a miss is a tolerated no-op.
        let Some(range) = location.range() else {
            return Ok(Vec::new());
        };
        Ok(state.covering(range.clone()).unwrap_or_default())
    }
}

impl<A: Send + Sync + 'static> Stream<Text> for ExtractStream<A> {
    fn format(&self) -> FormatId {
        self.format_id
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        let state = &self.state;
        Ok(state
            .passages()
            .into_iter()
            .map(|passage| {
                let start = state.item_starts[passage.start];
                let end = state.item_starts[passage.end];
                let items = &state.items[passage];
                Chunk {
                    location: TextLocation::inline(start..end),
                    data: TextData::new(
                        items
                            .iter()
                            .map(|item| item.value.as_str())
                            .collect::<String>(),
                    ),
                    hints: items
                        .iter()
                        .flat_map(|item| item.hints.iter().cloned())
                        .collect(),
                }
            })
            .collect())
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // Items are byte-for-byte the recognizer's view, so lifting is an
        // identity offset add of the chunk-local range against the chunk's
        // start, bounded by its end. Both are decoded coordinates (a chunk yields
        // a decoded range; a recognizer's local finding is a decoded offset).
        let chunk_range = chunk.location.range()?;
        let local_range = local.range()?;
        let base = chunk_range.start;
        let start = base.checked_add(local_range.start)?;
        let end = base.checked_add(local_range.end)?;
        if start > end || end > chunk_range.end {
            return None;
        }
        Some(self.source_location(start..end))
    }

    fn flagged(&self) -> Result<Vec<FlaggedItem<Text>>> {
        let flagged: Vec<_> = {
            let state = &self.state;
            state
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.passage != Passage::Separator)
                .filter_map(|(index, item)| {
                    let flag = item.flag.clone()?;
                    let range = state.item_starts[index]..state.item_starts[index + 1];
                    (!range.is_empty()).then_some((range, flag))
                })
                .collect()
        };
        Ok(flagged
            .into_iter()
            .map(|(range, flag)| {
                FlaggedItem::new(self.source_location(range), flag.label, flag.kind)
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl<A: Send + Sync + 'static> DataReader<Text> for ExtractStream<A> {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        // Reads address the decoded stream; a source-only location has no decoded
        // range to read from.
        let Some(range) = location.range() else {
            return Ok(None);
        };
        let state = &self.state;
        let Some(edits) = state.covering(range.clone()) else {
            return Ok(None);
        };
        let text: Option<String> = edits
            .into_iter()
            .map(|edit| state.items[edit.item].value.get(edit.local))
            .collect();
        Ok(text.map(TextData::new))
    }
}

#[async_trait::async_trait]
impl<A: Send + Sync + 'static> DataWriter<Text> for ExtractStream<A> {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        // Apply right-to-left so each edit's length delta doesn't
        // invalidate earlier locations.
        redactions.sort_by_position();
        for (location, replacement) in redactions.into_iter().rev() {
            Self::redact_one(&*self.addresser, &mut self.state, &location, &replacement)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::DataReader;

    use super::*;

    /// Addresses nothing: the items carry no source pre-image.
    struct Unaddressed;

    impl SourceAddresser<()> for Unaddressed {}

    fn item(value: &str, passage: Passage) -> ExtractedItem<()> {
        ExtractedItem {
            address: (),
            value: value.to_owned(),
            hints: Vec::new(),
            passage,
            flag: None,
        }
    }

    /// Two passages: a name and an email split across items around a line
    /// break, then a passage of its own.
    fn stream() -> ExtractStream<()> {
        let items = vec![
            item("Jane R", Passage::Start),
            item("oe", Passage::Continue),
            item("\n", Passage::Separator),
            item("jane.roe@", Passage::Continue),
            item("example.com", Passage::Continue),
            item("Next", Passage::Start),
        ];
        ExtractStream::new(FormatId::new("test"), items, Unaddressed)
    }

    fn values(stream: &ExtractStream<()>) -> Vec<String> {
        stream
            .items()
            .iter()
            .map(|item| item.value.clone())
            .collect()
    }

    #[test]
    fn each_passage_is_one_chunk() {
        let stream = stream();
        let chunks = stream.chunks().unwrap();
        let texts: Vec<&str> = chunks.iter().map(|chunk| chunk.data.as_str()).collect();
        assert_eq!(texts, ["Jane Roe\njane.roe@example.com", "Next"]);
        assert_eq!(chunks[1].location.range(), Some(&(29..33)));
    }

    #[tokio::test]
    async fn a_redaction_across_items_replaces_in_the_first_and_cuts_the_rest() {
        let mut stream = stream();
        // "Jane Roe\njane.roe@example.com": the whole passage, across the break.
        stream
            .write_at(Redactions::from([(
                TextLocation::inline(0..29),
                TextReplacement::substituted("[X]"),
            )]))
            .await
            .unwrap();
        assert_eq!(values(&stream), ["[X]", "", "\n", "", "", "Next"]);
    }

    #[tokio::test]
    async fn a_redaction_within_a_passage_keeps_the_separator() {
        let mut stream = stream();
        // "Roe": the tail of one item and the whole of the next.
        stream
            .write_at(Redactions::from([
                (
                    TextLocation::inline(5..8),
                    TextReplacement::substituted("[NAME]"),
                ),
                (
                    TextLocation::inline(9..29),
                    TextReplacement::substituted("[EMAIL]"),
                ),
            ]))
            .await
            .unwrap();
        assert_eq!(
            values(&stream),
            ["Jane [NAME]", "", "\n", "[EMAIL]", "", "Next"]
        );
    }

    #[tokio::test]
    async fn a_read_spans_items() {
        let stream = stream();
        let read = stream.read_at(&TextLocation::inline(5..18)).await.unwrap();
        assert_eq!(read.as_ref().map(TextData::as_str), Some("Roe\njane.roe@"));
    }
}
