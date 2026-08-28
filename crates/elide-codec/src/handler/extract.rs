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

use std::ops::Range;

use elide_core::modality::text::{SourceRef, Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter, Hint};
use elide_core::operator::Redactions;
use elide_core::{Error, ErrorKind, Result};

use crate::codec::Container;
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
    /// Out-of-band located context surfaced from the item's structural
    /// neighbours (e.g. a sibling element's text), each carrying the source
    /// span where its text sits. Empty when there's no useful surrounding
    /// context.
    pub hints: Vec<Hint<Text>>,
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

    /// The exact raw source byte range(s) that `local` (a byte range within
    /// `item`'s decoded value) came from, when the encoder addresses items by a
    /// source span.
    ///
    /// Usually one range, but a decoded range that crosses an entity
    /// substitution (a DOCX `&amp;`) maps back to several non-contiguous raw
    /// runs, so the return is a `Vec`. Empty means the encoder has no exact
    /// source pre-image to offer, the default: an address that is not a source
    /// byte span (an ordinal node index) cannot map back. The markup encoders
    /// return 0-or-1 — their value *is* the raw slice, so the mapping is an
    /// offset add; the DOCX encoder returns 1-or-more via its per-block offset
    /// map.
    fn source_span(
        &self,
        _item: &ExtractedItem<Self::Address>,
        _local: Range<usize>,
    ) -> Vec<SourceRef> {
        Vec::new()
    }

    /// Reverse of [`source_span`](Self::source_span): locate the item and the
    /// decoded-local byte range that the raw `source` references address.
    ///
    /// This is what lets a redaction target a span the caller has only in raw
    /// source coordinates — e.g. an entity a review layer added by selecting
    /// text in a container part, which it can express as part byte spans but not
    /// as a decoded-stream offset. `source` is the entity's whole
    /// [`SourceRef`](TextLocation::source) list, which must resolve to one
    /// contiguous decoded range within a single item (the runs of one selection
    /// share an item). Returns that item's index and the decoded-local range to
    /// edit, or `None` when the references do not resolve to a single item (or
    /// the encoder does not address items by source span — the default). The
    /// returned range feeds the same edit path a decoded-range redaction uses.
    fn locate_source(
        &self,
        _items: &[ExtractedItem<Self::Address>],
        _source: &[SourceRef],
    ) -> Option<(usize, Range<usize>)> {
        None
    }

    /// This encoder as a [`Container`] of cross-modality sub-parts, if the
    /// format is a container (DOCX). The default is `None`; a plain
    /// single-part encoder (HTML, XML) is not a container. Lives on the
    /// encoder because that is where a container format holds its retained
    /// package and part replacements.
    ///
    /// [`Container`]: crate::codec::Container
    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        None
    }
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
            *s = s.saturating_add_signed(delta);
        }
    }

    fn redact_one(&mut self, location: &TextLocation, replacement: &TextReplacement) -> Result<()> {
        // Resolve the edit to (item index, decoded-local range). A location
        // addressed by a raw `source` reference — e.g. an entity a review layer
        // added by selecting text in a container part — is reverse-resolved
        // through the encoder; otherwise the decoded `range` locates the item
        // directly. Either way the edit path below is the same.
        let Some((i, local_start, local_end)) = self.resolve(location)? else {
            return Ok(());
        };
        let value = replacement.value().unwrap_or_default();
        let before_len = self.items[i].value.len();
        redact::replace_range(&mut self.items[i].value, value, local_start..local_end)?;
        let delta = self.items[i].value.len() as isize - before_len as isize;
        self.shift_starts_after(i, delta);
        Ok(())
    }

    /// Resolve `location` to `(item index, decoded-local start, end)`.
    ///
    /// Prefers a raw [`source`](TextLocation::source) reference when present
    /// (reverse-resolved via [`Encoder::locate_source`]); falls back to the
    /// decoded [`range`](TextLocation::range).
    ///
    /// An unresolvable **`source`** reference is a caller mistake, not a no-op:
    /// the caller explicitly supplied raw byte coordinates to redact, and a bad
    /// part name, an offset past the part, or a cross-part span means those
    /// bytes were never redacted. Returning `Ok(None)` there would let a green
    /// audit stand over an unredacted document, so it is an `Err` instead. A
    /// missed decoded `range` stays `Ok(None)`: the pipeline's own coordinate,
    /// tolerated as a no-op.
    fn resolve(&self, location: &TextLocation) -> Result<Option<(usize, usize, usize)>> {
        if !location.source.is_empty() {
            let (i, local) = self
                .encoder
                .locate_source(&self.items, &location.source)
                .ok_or_else(|| unresolvable_source(&location.source))?;
            return Ok(Some((i, local.start, local.end)));
        }
        let Some(i) = self.item_for(location.range.start) else {
            return Ok(None);
        };
        let item_start = self.item_starts[i];
        let item_end = self.item_starts[i + 1];
        if location.range.end > item_end {
            return Ok(None);
        }
        Ok(Some((
            i,
            location.range.start - item_start,
            location.range.end - item_start,
        )))
    }
}

/// A location's raw [`source`](TextLocation::source) reference could not be
/// reverse-resolved to any redactable item — a bad part name, an offset past
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
            location: TextLocation::new(start, end),
            data,
            hints,
        }))
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // Items are byte-for-byte the recognizer's view, so lifting is an
        // identity offset add of the chunk-local range against the chunk's
        // start, bounded by its end.
        let base = chunk.location.range.start;
        let start = base.checked_add(local.range.start)?;
        let end = base.checked_add(local.range.end)?;
        if start > end || end > chunk.location.range.end {
            return None;
        }
        // The exact raw source range(s) this finding came from, when the encoder
        // can map them (markup, whose item value is a verbatim source slice;
        // DOCX, via its per-block offset map). Empty when it cannot.
        let source = self
            .item_for(base)
            .map(|i| {
                self.encoder
                    .source_span(&self.items[i], local.range.start..local.range.end)
            })
            .unwrap_or_default();
        Some(
            TextLocation::new(start, end)
                .with_page(chunk.location.page)
                .with_source(source),
        )
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        self.encoder.as_container_mut()
    }
}

#[async_trait::async_trait]
impl<E: Encoder> DataReader<Text> for ExtractHandler<E> {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(i) = self.item_for(location.range.start) else {
            return Ok(None);
        };
        let item_start = self.item_starts[i];
        let item_end = self.item_starts[i + 1];
        if location.range.end > item_end {
            return Ok(None);
        }
        let local_start = location.range.start - item_start;
        let local_end = location.range.end - item_start;
        Ok(self.items[i]
            .value
            .get(local_start..local_end)
            .map(TextData::new))
    }
}

#[async_trait::async_trait]
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
