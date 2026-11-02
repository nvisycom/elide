//! The item stream's two ends: [`MarkupSource`] queries the retained source
//! document (slicing ranges, locating attribute value spans), and [`MarkupSink`]
//! accumulates the extracted item stream, keeping its engine-space offset in
//! step so a chunk's offset can never drift from the stream it indexes.

use std::borrow::Cow;
use std::ops::Range;

use elide_codec::extract::{ExtractedItem, Passage};
use elide_core::modality::ResolvedHint;
use elide_core::modality::text::Text;
use elide_core::{Error, Result};
use quick_xml::events::BytesStart;

use super::tag::HtmlTagExt;
use super::xml_handler::{XmlItem, XmlSpan};

/// One redactable attribute's source spans: its `value` (the redactable inner
/// bytes) and its `name` (the located context hint), the latter `None` when the
/// name could not be placed in the source.
pub(super) struct AttributeSpan {
    pub(super) name: Option<Range<usize>>,
    pub(super) value: Range<usize>,
}

/// A start tag as the engine reads it.
pub(super) struct StartTag {
    /// The name, lowercased for matching.
    pub(super) name: String,
    /// The name as written, for its context hint.
    pub(super) written: String,
    /// The redactable attributes, in source order.
    pub(super) attributes: Vec<AttributeSpan>,
}

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

    /// The whole retained source document.
    pub(super) fn raw(&self) -> &'a str {
        self.raw
    }

    /// The source text over `range`.
    pub(super) fn slice(&self, range: Range<usize>) -> &'a str {
        &self.raw[range]
    }

    /// The start tag `e` as the engine reads it: its name, and each redactable
    /// attribute with the source span of its name (a located context hint)
    /// and of its value, the inner bytes between the quotes, as written. The
    /// name lets a context-gated value (`ssn="…"`) be boosted by its attribute
    /// name.
    ///
    /// HTML reads the tag as a browser does ([`HtmlTagExt`]): unquoted and
    /// missing values, and a `/` between attributes, included. Strict XML
    /// validates attributes (rejecting e.g. a duplicate key or an unquoted
    /// value) and reports an [`AttrError`] as
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput).
    /// An attribute without a value, or with only whitespace, has nothing to
    /// redact and is left out.
    ///
    /// [`AttrError`]: quick_xml::events::attributes::AttrError
    pub(super) fn start_tag(&self, e: &BytesStart<'_>, html: bool) -> Result<StartTag> {
        let mut attributes = Vec::new();
        if html {
            let tag = e.html5_tag(self.raw)?;
            for attr in &tag.attributes {
                if let Some(value) = attr.value {
                    self.push_attribute(&mut attributes, attr.name, value)?;
                }
            }
            return Ok(StartTag {
                name: tag.name.to_ascii_lowercase(),
                written: tag.name.to_owned(),
                attributes,
            });
        }
        for attr in e.attributes() {
            let attr = attr.map_err(|e| Error::malformed(format!("malformed attribute: {e}")))?;
            // Over a `Reader::from_str`, the key and the raw value (entity
            // spellings intact) borrow straight out of the source buffer.
            let Cow::Borrowed(value) = attr.value else {
                return Err(Error::malformed(
                    "an XML attribute value is not in the source",
                ));
            };
            self.push_attribute(&mut attributes, AsRef::<str>::as_ref(&attr.key), value)?;
        }
        let written = AsRef::<str>::as_ref(&e.local_name()).to_owned();
        Ok(StartTag {
            name: written.to_ascii_lowercase(),
            written,
            attributes,
        })
    }

    /// Push the attribute `name` with `value`, both borrowed from this source,
    /// unless its value is blank. Fails closed when the value cannot be placed
    /// in the source, which would leave it unredactable.
    fn push_attribute(&self, out: &mut Vec<AttributeSpan>, name: &str, value: &str) -> Result<()> {
        let value_span = self
            .slice_span(value)
            .ok_or_else(|| Error::malformed("an attribute value is not in the source"))?;
        if value.trim().is_empty() {
            return Ok(());
        }
        out.push(AttributeSpan {
            name: self.slice_span(name),
            value: value_span,
        });
        Ok(())
    }

    /// The byte range a `slice` borrowed out of this source occupies in it, or
    /// `None` if it is not a valid in-bounds char-aligned subslice.
    fn slice_span(&self, slice: &str) -> Option<Range<usize>> {
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
/// the two in step, every item advances the offset by its own length, so the
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

    /// Append an item for `value` read at `address`, joining the passage
    /// before it as `passage` says, and advance the offset by the value's
    /// length. Returns the new item's index.
    pub(super) fn push(&mut self, (value, address): (String, XmlSpan), passage: Passage) -> usize {
        self.offset += value.len();
        let index = self.items.len();
        self.items.push(ExtractedItem {
            value,
            address,
            hints: Vec::new(),
            passage,
        });
        index
    }

    /// Attach hints to items, each pair naming an item index and its hints.
    /// Empty hint lists are ignored.
    pub(super) fn apply_hints(
        &mut self,
        hints: impl IntoIterator<Item = (usize, Vec<ResolvedHint<Text>>)>,
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
