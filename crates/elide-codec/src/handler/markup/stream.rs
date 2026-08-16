//! The item stream's two ends: [`MarkupSource`] queries the retained source
//! document (slicing ranges, locating attribute value spans), and [`MarkupSink`]
//! accumulates the extracted item stream, keeping its engine-space offset in
//! step so a chunk's offset can never drift from the stream it indexes.

use std::borrow::Cow;
use std::ops::Range;

use elide_core::modality::Hint;
use elide_core::modality::text::Text;
use elide_core::{Error, ErrorKind, Result};
use quick_xml::events::BytesStart;

use super::xml_handler::{XmlItem, XmlSpan};
use crate::handler::extract::ExtractedItem;

/// The retained source document, read-only. Owns the span queries the engine
/// runs against it: slicing a range, finding an attribute value's source span,
/// and dropping whitespace-only spans.
#[derive(Clone, Copy)]
pub(super) struct MarkupSource<'a> {
    raw: &'a str,
}

impl<'a> MarkupSource<'a> {
    pub(super) fn new(raw: &'a str) -> Self {
        Self { raw }
    }

    /// The source text over `range`.
    pub(super) fn slice(&self, range: Range<usize>) -> &'a str {
        &self.raw[range]
    }

    /// `span` unless it covers only whitespace (blank runs carry no PII and only
    /// clutter the stream).
    pub(super) fn non_blank(&self, span: Range<usize>) -> Option<Range<usize>> {
        (!self.raw[span.clone()].trim().is_empty()).then_some(span)
    }

    /// The source byte spans of an element's redactable attribute values — the
    /// inner bytes between the quotes. Values pass through verbatim, so a
    /// `mailto:` URL has its email matched in place.
    ///
    /// Strict XML validates attributes (rejecting e.g. a duplicate key) and
    /// reports an [`AttrError`] as [`ErrorKind::MalformedInput`]; lenient HTML
    /// turns validation off and skips an unparseable attribute rather than
    /// failing.
    ///
    /// [`AttrError`]: quick_xml::events::attributes::AttrError
    pub(super) fn attribute_spans(
        &self,
        e: &BytesStart<'_>,
        lenient: bool,
    ) -> Result<Vec<Range<usize>>> {
        let mut spans = Vec::new();
        for attr in e.attributes().with_checks(!lenient) {
            let attr = match attr {
                Ok(attr) => attr,
                // Lenient parsing tolerates a malformed attribute; strict XML
                // does not — a duplicate key or unquoted value is malformed.
                Err(_) if lenient => continue,
                Err(e) => {
                    return Err(Error::new(
                        ErrorKind::MalformedInput,
                        format!("malformed attribute: {e}"),
                    ));
                }
            };
            // `Attribute::value` is the *raw* on-the-wire bytes, entity spellings
            // intact (decoding happens only through `unescape_value`, which we
            // never call — the engine redacts raw slices). Over a
            // `Reader::from_str`, the value borrows straight out of the source
            // buffer, so its slice position *is* the source span — even for an
            // entity-bearing value. `Cow::Owned` only arises for a non-borrowed
            // value (no source position); it is a defensive guard, and such an
            // attribute is left un-redactable rather than guessed at.
            let Cow::Borrowed(bytes) = attr.value else {
                continue;
            };
            let Some(inner) = self.slice_span(bytes) else {
                continue;
            };
            if self.raw[inner.clone()].trim().is_empty() {
                continue;
            }
            spans.push(inner);
        }
        Ok(spans)
    }

    /// The byte range a `slice` borrowed out of this source occupies in it, or
    /// `None` if it is not a valid in-bounds char-aligned subslice.
    fn slice_span(&self, slice: &[u8]) -> Option<Range<usize>> {
        let base = self.raw.as_ptr() as usize;
        let start = (slice.as_ptr() as usize).checked_sub(base)?;
        let end = start.checked_add(slice.len())?;
        (end <= self.raw.len()
            && self.raw.is_char_boundary(start)
            && self.raw.is_char_boundary(end))
        .then_some(start..end)
    }
}

/// The growing item stream: the extracted items and the running engine-space
/// offset (the cumulative length of their values). [`push`](Self::push) keeps
/// the two in step — every item advances the offset by its own length — so the
/// offset a chunk carries can never drift from the stream it indexes.
pub(super) struct MarkupSink {
    items: Vec<XmlItem>,
    offset: usize,
}

impl MarkupSink {
    pub(super) fn new() -> Self {
        Self {
            items: Vec::new(),
            offset: 0,
        }
    }

    /// The current engine-space offset (where the next item's value begins).
    pub(super) fn offset(&self) -> usize {
        self.offset
    }

    /// Append an item for `value` at source `span`, advancing the offset by the
    /// value's length. Returns the new item's index.
    pub(super) fn push(&mut self, value: String, span: Range<usize>) -> usize {
        self.offset += value.len();
        let index = self.items.len();
        self.items.push(ExtractedItem {
            value,
            address: XmlSpan(span),
            hints: Vec::new(),
        });
        index
    }

    /// Attach hints to items, each pair naming an item index and its hints.
    /// Empty hint lists are ignored.
    pub(super) fn apply_hints(
        &mut self,
        hints: impl IntoIterator<Item = (usize, Vec<Hint<Text>>)>,
    ) {
        for (index, hints) in hints {
            if !hints.is_empty() {
                self.items[index].hints = hints;
            }
        }
    }

    /// The finished item stream.
    pub(super) fn into_items(self) -> Vec<XmlItem> {
        self.items
    }
}
