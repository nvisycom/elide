//! Shared model for tree-structured markup (HTML, and a future XML).
//!
//! Markup formats differ in their *parser* and *serializer* but share the
//! same redactable units — text nodes, element attributes, comments — and
//! the same streaming/redaction bookkeeping over them. This module holds
//! that neutral core:
//!
//! - [`RedactableItem<A>`] — one addressable unit (its `value` plus an
//!   encoder-private address `A`), parser-agnostic.
//! - [`MarkupHandler`] — the [`Handler`] machinery over a
//!   `Vec<RedactableItem<A>>`: cumulative offsets, `read_next`, random
//!   read, batch redact, and `lift_chunk`. It never inspects the address.
//!   Re-serialization is delegated to a format-specific [`MarkupEncoder`],
//!   which also chooses the [`Address`] type.
//!
//! A concrete format (e.g. the `html_loader` / `html_handler` pair in
//! this module) supplies a parser that produces the item stream and a
//! [`MarkupEncoder`] that splices mutated values back into its native
//! tree; everything between is shared. A future XML format would add an
//! `xml_loader` / `xml_handler` pair alongside.
//!
//! [`RedactableItem<A>`]: RedactableItem
//! [`Address`]: MarkupEncoder::Address

#[cfg(feature = "html")]
mod html_handler;
#[cfg(feature = "html")]
mod html_loader;
#[cfg(feature = "xml")]
mod xml_handler;
#[cfg(feature = "xml")]
mod xml_loader;

use std::ops::Range;

use elide_core::Error;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

#[cfg(feature = "html")]
pub use self::html_handler::{HtmlEncoder, HtmlHandler, format as html_format};
#[cfg(feature = "html")]
pub use self::html_loader::{HtmlLoader, ScriptPolicy};
#[cfg(feature = "xml")]
pub use self::xml_handler::{XmlEncoder, XmlHandler, format as xml_format};
#[cfg(feature = "xml")]
pub use self::xml_loader::XmlLoader;
use crate::content::ContentData;
use crate::handler::redact;
use crate::Handler;

/// One redactable unit in a markup document.
///
/// `value` is the text a recognizer scans and that redaction mutates in
/// place; `address` is the encoder-private "where" — how the format's
/// [`MarkupEncoder`] re-finds this unit to splice the mutated value back
/// in. The handler machinery never inspects `address`; it only streams
/// and edits `value`, so each format chooses the addressing scheme its
/// encoder needs (ordinal node indices for a DOM rebuild, source byte
/// spans for in-place patching, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactableItem<A> {
    /// The encoder-private location of this item in the document.
    pub address: A,
    /// Text-node text, comment body, attribute value, or element text.
    pub value: String,
    /// Out-of-band context strings surfaced from the item's structural
    /// neighbours (e.g. the parent element's text minus this item's own).
    /// Empty when there's no useful surrounding context.
    pub hints: Vec<String>,
}

/// Re-serialize a mutated [`RedactableItem`] stream into a document's
/// native bytes.
///
/// A markup format implements this over its own parser/serializer: it
/// chooses an [`Address`] type for locating items, and `encode` splices
/// each item's current `value` back at its address and emits.
/// [`MarkupHandler`] owns everything else.
///
/// [`Address`]: MarkupEncoder::Address
pub trait MarkupEncoder: Send + Sync + 'static {
    /// The encoder-private addressing payload carried on each
    /// [`RedactableItem`] — e.g. an ordinal node index or a source span.
    type Address: Send + Sync + 'static;

    /// Re-encode `items` against the retained source into output bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the document cannot be re-serialized.
    fn encode(&self, items: &[RedactableItem<Self::Address>]) -> Result<ContentData, Error>;
}

/// The [`Handler`] machinery over a markup item stream.
///
/// `item_starts` is a cumulative-offset index over the items:
/// `item_starts[i]` is the byte position of item `i` in the concatenated
/// item-value stream, and `item_starts[items.len()]` is the total-length
/// sentinel. Maintained on every redaction so random-access reads run in
/// `O(log N)`. Offsets are over the redactable-item sequence in document
/// order, not raw source bytes.
#[derive(Debug)]
pub struct MarkupHandler<E: MarkupEncoder> {
    format_id: crate::FormatId,
    encoder: E,
    items: Vec<RedactableItem<E::Address>>,
    item_starts: Vec<usize>,
    cursor: usize,
}

impl<E: MarkupEncoder> MarkupHandler<E> {
    /// Build a handler from a decoded item stream, a format id, and the
    /// format's encoder.
    pub fn new(
        format_id: crate::FormatId,
        encoder: E,
        items: Vec<RedactableItem<E::Address>>,
    ) -> Self {
        let item_starts = compute_item_starts(&items);
        Self {
            format_id,
            encoder,
            items,
            item_starts,
            cursor: 0,
        }
    }

    /// All redactable items in document order.
    pub fn items(&self) -> &[RedactableItem<E::Address>] {
        &self.items
    }

    /// Total number of redactable items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the document has no redactable items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Rewind the streaming cursor to the start of the document.
    pub fn rewind(&mut self) {
        self.cursor = 0;
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

    fn redact_one(
        &mut self,
        location: &TextLocation,
        replacement: &elide_core::modality::text::TextReplacement,
    ) -> Result<(), Error> {
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

impl<E: MarkupEncoder> Handler<Text> for MarkupHandler<E> {
    fn format(&self) -> crate::FormatId {
        self.format_id.clone()
    }

    fn encode(&self) -> Result<ContentData, Error> {
        self.encoder.encode(&self.items)
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>, Error> {
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

    fn lift_chunk(&self, chunk: &Chunk<Text>, value_range: Range<usize>) -> Option<TextLocation> {
        // Items are byte-for-byte the recognizer's view, so lifting is an
        // identity offset add against the chunk's start, bounded by its
        // end.
        let base = chunk.location.start;
        let start = base + value_range.start;
        let end = base + value_range.end;
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

impl<E: MarkupEncoder> DataReader<Text> for MarkupHandler<E> {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>, Error> {
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

impl<E: MarkupEncoder> DataWriter<Text> for MarkupHandler<E> {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<(), Error> {
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
fn compute_item_starts<A>(items: &[RedactableItem<A>]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(items.len() + 1);
    let mut offset = 0usize;
    for item in items {
        starts.push(offset);
        offset += item.value.len();
    }
    starts.push(offset);
    starts
}
