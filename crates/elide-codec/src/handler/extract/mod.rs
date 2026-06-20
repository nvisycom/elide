//! Shared text-extract-and-splice engine for structured formats.
//!
//! Many formats — markup (HTML, XML) and, ahead, rich documents (RTF,
//! DOCX) — differ in their *parser* and *serializer* but share the same
//! redactable shape: a sequence of text-valued units, each carrying an
//! encoder-private address, redacted as text and spliced back into the
//! native container. This module is that neutral core:
//!
//! - [`ExtractedItem<A>`]: one addressable unit (its `value` plus an
//!   encoder-private address `A`), parser-agnostic.
//! - [`ExtractHandler`]: the [`Handler`] machinery over a
//!   `Vec<ExtractedItem<A>>`: cumulative offsets, `read_next`, random
//!   read, batch redact, and `lift`. It never inspects the address.
//!   Re-serialization is delegated to a format-specific [`Encoder`],
//!   which also chooses the [`Address`] type.
//!
//! A concrete format supplies a parser that produces the item stream and
//! an [`Encoder`] that splices mutated values back into its native bytes;
//! everything between is shared. The item value is always [`Text`], so a
//! recognizer or operator written for text serves every format built on
//! this engine unchanged.
//!
//! [`Handler`]: crate::Handler
//! [`Address`]: Encoder::Address

use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use crate::content::ContentData;
use crate::handler::redact;
use crate::{FormatId, Handler};

/// One redactable unit in a structured document.
///
/// `value` is the text a recognizer scans and that redaction mutates in
/// place; `address` is the encoder-private "where": how the format's
/// [`Encoder`] re-finds this unit to splice the mutated value back in. The
/// handler machinery never inspects `address`; it only streams and edits
/// `value`, so each format chooses the addressing scheme its encoder needs
/// (ordinal node indices for a DOM rebuild, source byte spans for in-place
/// patching, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedItem<A> {
    /// The encoder-private location of this item in the document.
    pub address: A,
    /// Text-node text, comment body, attribute value, or element text.
    pub value: String,
    /// Out-of-band context strings surfaced from the item's structural
    /// neighbours (e.g. the parent element's text minus this item's own).
    /// Empty when there's no useful surrounding context.
    pub hints: Vec<String>,
}

/// Re-serialize a mutated [`ExtractedItem`] stream into a document's
/// native bytes.
///
/// A format implements this over its own parser/serializer: it chooses an
/// [`Address`] type for locating items, and `encode` splices each item's
/// current `value` back at its address and emits. [`ExtractHandler`] owns
/// everything else.
///
/// [`Address`]: Encoder::Address
pub(crate) trait Encoder: Send + Sync + 'static {
    /// The encoder-private addressing payload carried on each
    /// [`ExtractedItem`], e.g. an ordinal node index or a source span.
    type Address: Send + Sync + 'static;

    /// Re-encode `items` against the retained source into output bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the document cannot be re-serialized.
    fn encode(&self, items: &[ExtractedItem<Self::Address>]) -> Result<ContentData>;
}

/// The [`Handler`] machinery over an extracted item stream.
///
/// `item_starts` is a cumulative-offset index over the items:
/// `item_starts[i]` is the byte position of item `i` in the concatenated
/// item-value stream, and `item_starts[items.len()]` is the total-length
/// sentinel. Maintained on every redaction so random-access reads run in
/// `O(log N)`. Offsets are over the redactable-item sequence in document
/// order, not raw source bytes.
#[derive(Debug)]
pub(crate) struct ExtractHandler<E: Encoder> {
    format_id: FormatId,
    encoder: E,
    items: Vec<ExtractedItem<E::Address>>,
    item_starts: Vec<usize>,
    cursor: usize,
}

impl<E: Encoder> ExtractHandler<E> {
    /// Build a handler from a decoded item stream, a format id, and the
    /// format's encoder.
    pub fn new(format_id: FormatId, encoder: E, items: Vec<ExtractedItem<E::Address>>) -> Self {
        let item_starts = compute_item_starts(&items);
        Self {
            format_id,
            encoder,
            items,
            item_starts,
            cursor: 0,
        }
    }

    fn item_for(&self, byte_offset: usize) -> Option<usize> {
        match self.item_starts.binary_search(&byte_offset) {
            Ok(i) if i < self.items.len() => Some(i),
            Ok(_) => None,
            Err(i) if i > 0 && i <= self.items.len() => Some(i - 1),
            _ => None,
        }
    }

    fn shift_starts_after(&mut self, i: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for s in &mut self.item_starts[i + 1..] {
            *s = (*s as isize + delta) as usize;
        }
    }

    fn redact_one(&mut self, location: &TextLocation, replacement: &TextReplacement) -> Result<()> {
        let Some(i) = self.item_for(location.start) else {
            return Ok(());
        };
        let item_start = self.item_starts[i];
        let item_end = self.item_starts[i + 1];
        if location.end > item_end {
            return Ok(());
        }
        let local_start = location.start - item_start;
        let local_end = location.end - item_start;
        let value = replacement.value().unwrap_or_default();
        let before_len = self.items[i].value.len();
        redact::replace_range(&mut self.items[i].value, value, local_start..local_end)?;
        let delta = self.items[i].value.len() as isize - before_len as isize;
        self.shift_starts_after(i, delta);
        Ok(())
    }
}

impl<E: Encoder> Handler<Text> for ExtractHandler<E> {
    fn format(&self) -> FormatId {
        self.format_id.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        self.encoder.encode(&self.items)
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        if self.cursor >= self.items.len() {
            return Ok(None);
        }
        let i = self.cursor;
        let start = self.item_starts[i];
        let end = self.item_starts[i + 1];
        let item = &self.items[i];
        let data = TextData::new(item.value.clone());
        let hints = item.hints.clone();
        self.cursor += 1;
        Ok(Some(Chunk {
            location: TextLocation {
                start,
                end,
                ..Default::default()
            },
            data,
            hints,
        }))
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // Items are byte-for-byte the recognizer's view, so lifting is an
        // identity offset add of the chunk-local range against the chunk's
        // start, bounded by its end.
        let base = chunk.location.start;
        let start = base + local.start;
        let end = base + local.end;
        if start > end || end > chunk.location.end {
            return None;
        }
        Some(TextLocation {
            start,
            end,
            page: chunk.location.page,
        })
    }
}

impl<E: Encoder> DataReader<Text> for ExtractHandler<E> {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(i) = self.item_for(location.start) else {
            return Ok(None);
        };
        let item_start = self.item_starts[i];
        let item_end = self.item_starts[i + 1];
        if location.end > item_end {
            return Ok(None);
        }
        let local_start = location.start - item_start;
        let local_end = location.end - item_start;
        Ok(self.items[i]
            .value
            .get(local_start..local_end)
            .map(TextData::new))
    }
}

impl<E: Encoder> DataWriter<Text> for ExtractHandler<E> {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        // Apply right-to-left so each edit's length delta doesn't
        // invalidate earlier locations.
        redactions.sort_by_position();
        for (location, replacement) in redactions.into_iter().rev() {
            self.redact_one(&location, &replacement)?;
        }
        Ok(())
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
