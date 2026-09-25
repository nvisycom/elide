//! [`ExtractStream<A>`]: the [`Stream`](crate::Stream) machinery over a shared
//! extracted item stream.

use std::sync::Arc;

use elide_core::modality::text::{SourceRef, Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

use super::{ItemEdit, SharedSplice, SourceAddresser, SpliceState};
use crate::content::ContentData;
use crate::string::RedactRange;
use crate::{FormatId, Stream};

/// The [`Stream`](crate::Stream) machinery over a shared extracted item stream.
///
/// It streams each item's `value` as a [`Chunk`], reads/redacts by decoded
/// offset, and lifts a chunk-local finding to source coordinates via the
/// [`SourceAddresser`]. The decoded items live in a [`SharedSplice`] the
/// format's [`Recombine`](crate::Recombine) also holds, so a redaction here is
/// visible when the document re-serialises. `cursor` is per-stream (not shared):
/// it is this reader's position, not document state.
///
/// [`Chunk`]: elide_core::modality::Chunk
pub struct ExtractStream<A> {
    format_id: FormatId,
    state: SharedSplice<A>,
    addresser: Arc<dyn SourceAddresser<A>>,
    cursor: usize,
}

impl<A: Send + Sync + 'static> ExtractStream<A> {
    /// Build a stream over `state`, addressed by `addresser`.
    pub fn new(
        format_id: FormatId,
        state: SharedSplice<A>,
        addresser: Arc<dyn SourceAddresser<A>>,
    ) -> Self {
        Self {
            format_id,
            state,
            addresser,
            cursor: 0,
        }
    }

    fn redact_one(
        &self,
        state: &mut SpliceState<A>,
        location: &TextLocation,
        replacement: &TextReplacement,
    ) -> Result<()> {
        // Resolve to the item and its decoded-local range. A location addressed
        // by a raw `source` reference, e.g. an entity a review layer added by
        // selecting text in a container part, is reverse-resolved through the
        // addresser; otherwise the decoded `range` locates the item directly.
        // Either way the edit path below is the same.
        let Some(ItemEdit { item, local }) = self.resolve(state, location)? else {
            return Ok(());
        };
        let value = replacement.value().unwrap_or_default();
        let before_len = state.items[item].value.len();
        state.items[item].value.redact_range(value, local)?;
        let delta = state.items[item].value.len() as isize - before_len as isize;
        state.shift_starts_after(item, delta);
        Ok(())
    }

    /// Resolve `location` to the [`ItemEdit`] it targets.
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
    /// missed decoded `range` stays `Ok(None)`: the pipeline's own coordinate,
    /// tolerated as a no-op.
    fn resolve(&self, state: &SpliceState<A>, location: &TextLocation) -> Result<Option<ItemEdit>> {
        if !location.source().is_empty() {
            let edit = self
                .addresser
                .locate_source(&state.items, location.source())
                .ok_or_else(|| unresolvable_source(location.source()))?;
            return Ok(Some(edit));
        }
        // No source refs and no decoded range: a source-only location with an
        // empty ref set, nothing to resolve. A decoded range is the pipeline's
        // own coordinate, a miss is a tolerated no-op.
        let Some(range) = location.range() else {
            return Ok(None);
        };
        let Some(item) = state.item_for(range.start) else {
            return Ok(None);
        };
        let item_start = state.item_starts[item];
        let item_end = state.item_starts[item + 1];
        if range.end > item_end {
            return Ok(None);
        }
        Ok(Some(ItemEdit {
            item,
            local: (range.start - item_start)..(range.end - item_start),
        }))
    }
}

/// A location's raw [`source`](TextLocation::source) reference could not be
/// reverse-resolved to any redactable item, a bad part name, an offset past
/// the part, or a span crossing parts. `MalformedInput`: the caller-supplied
/// coordinate is at fault.
fn unresolvable_source(source: &[SourceRef]) -> Error {
    let mut msg = String::from("source reference resolves to no redactable item:");
    for src in source {
        match &src.part {
            Some(part) => {
                msg.push_str(&format!(" {}#{}..{}", part, src.range.start, src.range.end))
            }
            None => msg.push_str(&format!(" {}..{}", src.range.start, src.range.end)),
        }
    }
    Error::new(ErrorKind::MalformedInput, msg)
}

#[async_trait::async_trait]
impl<A: Send + Sync + 'static> Stream<Text> for ExtractStream<A> {
    fn format(&self) -> FormatId {
        self.format_id.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        // The re-pack is the format's [`Recombine`], which holds the same shared
        // [`SpliceState`] and re-serialises the redacted items into the native
        // container (splicing markup, re-packing an OOXML zip). A spliced body
        // has no standalone bytes, so this stream's own encode is an ignored
        // marker (the document's `Recombine::assemble` discards this part's
        // `EncodedPart` bytes, exactly as the image codec ignores its pixel
        // stream's bytes and reads the shared buffer).
        Ok(ContentData::new(bytes::Bytes::new()))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        let state = self.state.lock();
        if self.cursor >= state.items.len() {
            return Ok(None);
        }
        let i = self.cursor;
        let start = state.item_starts[i];
        let end = state.item_starts[i + 1];
        let item = &state.items[i];
        let data = TextData::new(item.value.clone());
        let hints = item.hints.clone();
        self.cursor += 1;
        Ok(Some(Chunk {
            location: TextLocation::new(start, end),
            data,
            hints,
        }))
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
        // The exact raw source range(s) this finding came from, when the
        // addresser can map them (markup, whose item value is a verbatim source
        // slice; OOXML, via its per-block offset map). Empty when it cannot.
        let state = self.state.lock();
        let source = state
            .item_for(base)
            .map(|i| {
                self.addresser
                    .source_span(&state.items[i], local_range.start..local_range.end)
            })
            .unwrap_or_default();
        Some(
            TextLocation::new(start, end)
                .with_page(chunk.location.page)
                .with_source(source),
        )
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
        let state = self.state.lock();
        let Some(i) = state.item_for(range.start) else {
            return Ok(None);
        };
        let item_start = state.item_starts[i];
        let item_end = state.item_starts[i + 1];
        if range.end > item_end {
            return Ok(None);
        }
        let local_start = range.start - item_start;
        let local_end = range.end - item_start;
        Ok(state.items[i]
            .value
            .get(local_start..local_end)
            .map(TextData::new))
    }
}

#[async_trait::async_trait]
impl<A: Send + Sync + 'static> DataWriter<Text> for ExtractStream<A> {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        // Apply right-to-left so each edit's length delta doesn't
        // invalidate earlier locations.
        redactions.sort_by_position();
        let mut state = self.state.lock();
        for (location, replacement) in redactions.into_iter().rev() {
            self.redact_one(&mut state, &location, &replacement)?;
        }
        Ok(())
    }
}
