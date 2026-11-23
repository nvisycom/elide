//! [`MarkupSpan`]: where a markup item's value was read from and how a new value
//! is written back, and the [`MarkupAddresser`] that maps a decoded range to
//! and from the raw bytes through it.

use std::ops::Range;

use elide_codec::extract::{ExtractedItem, ItemEdit, OffsetMap, Passage, Rewrite, SourceAddresser};
use elide_core::modality::text::SourceRef;
use quick_xml::escape::{escape, partial_escape};

use super::percent::decode_percent;
use crate::references::{CharacterReferences, decode_references};

/// A markup [`ExtractedItem`] addressed by the source bytes its `value` was read
/// from.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
pub(crate) type MarkupItem = ExtractedItem<MarkupSpan>;

/// Where an [`ExtractedItem`]'s value was read from in the retained raw
/// document, and how a new value is written back there.
///
/// The value is the text a reader sees: a text run's or an attribute value's
/// character references (`&amp;`, `&#64;`, HTML's `&nbsp;`) are decoded, an
/// attribute value's percent-encoded characters (`%40`) after them, and
/// [`offsets`](Self::offsets) maps it back onto the raw bytes.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkupSpan {
    /// The *inner* raw bytes the value was read from: a text run, an
    /// attribute value between its quotes, a comment body between `<!--` and
    /// `-->`, a CDATA payload between `<![CDATA[` and `]]>`.
    pub(crate) range: Range<usize>,
    /// The map from the value back onto [`range`](Self::range).
    pub(crate) offsets: OffsetMap,
    /// How a new value is escaped where it lands.
    pub(crate) kind: SpanKind,
    /// The value as read, which a write-back compares the current value
    /// against.
    pub(crate) read: String,
}

/// How a new value is escaped where an [`MarkupSpan`] lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpanKind {
    /// Character data: `<`, `>`, and `&` are escaped.
    Text,
    /// An attribute value: quotes are escaped too, so the value cannot end
    /// early.
    Attribute,
    /// A comment body or CDATA payload, written as it is.
    Verbatim,
}

impl MarkupSpan {
    /// The span of the escaped `kind` text at `range` of `raw`, its references
    /// decoded by `references`, and an attribute value's percent-encoded
    /// characters too.
    pub(crate) fn escaped(
        raw: &str,
        range: Range<usize>,
        kind: SpanKind,
        references: CharacterReferences,
    ) -> Self {
        let read = decode_references(&raw[range.clone()], range.start, references);
        let read = match kind {
            SpanKind::Attribute => decode_percent(read),
            SpanKind::Text | SpanKind::Verbatim => read,
        };
        Self::new(range, read.offsets, kind, read.text)
    }

    /// The span of the verbatim text at `range` of `raw`.
    pub(crate) fn verbatim(raw: &str, range: Range<usize>) -> Self {
        let read = raw[range.clone()].to_owned();
        let offsets = OffsetMap::identity(range.start, range.len());
        Self::new(range, offsets, SpanKind::Verbatim, read)
    }

    /// The span of a separator reading as `value`, which stands for the markup
    /// at `range` and maps to no source text.
    pub(crate) fn separator(range: Range<usize>, value: &str) -> Self {
        Self::new(
            range,
            OffsetMap::default(),
            SpanKind::Verbatim,
            value.to_owned(),
        )
    }

    fn new(range: Range<usize>, offsets: OffsetMap, kind: SpanKind, read: String) -> Self {
        Self {
            range,
            offsets,
            kind,
            read,
        }
    }

    /// The raw range to replace, and its new raw text, that write `value` back
    /// over this span; `None` while `value` is as read.
    ///
    /// Only the stretch between the longest unchanged start and end is
    /// rewritten, widened to whole character references, so a reference the
    /// redaction did not touch keeps its spelling.
    pub(crate) fn write_back(&self, value: &str) -> Option<(Range<usize>, String)> {
        let Rewrite { raw, text } = self.offsets.rewrite(&self.read, value)?;
        let text = match self.kind {
            SpanKind::Text => partial_escape(text),
            SpanKind::Attribute => escape(text),
            SpanKind::Verbatim => text.into(),
        };
        Some((raw, text.into_owned()))
    }
}

/// Maps a markup item's decoded value range to/from its raw source bytes
/// through its [`MarkupSpan`]'s offset map, single-file (no part tag).
#[derive(Debug, Default)]
pub(crate) struct MarkupAddresser;

impl SourceAddresser<MarkupSpan> for MarkupAddresser {
    fn source_span(&self, item: &MarkupItem, local: Range<usize>) -> Vec<SourceRef> {
        item.address
            .offsets
            .raw_ranges(local)
            .into_iter()
            .map(SourceRef::new)
            .collect()
    }

    fn locate_source(&self, items: &[MarkupItem], source: &[SourceRef]) -> Vec<ItemEdit> {
        // XML is a single file, so its source references carry no part; reject
        // any part-tagged reference rather than misresolve it. Each reference
        // must lie within one item.
        let mut edits = Vec::with_capacity(source.len());
        for reference in source {
            let raw = &reference.range;
            let item = items.iter().enumerate().find(|(_, item)| {
                item.passage != Passage::Separator
                    && item.address.range.start <= raw.start
                    && raw.end <= item.address.range.end
            });
            let (None, Some((item, found))) = (&reference.part, item) else {
                return Vec::new();
            };
            let decoded = found.address.offsets.decoded_ranges(raw.clone());
            let (Some(first), Some(last)) = (decoded.first(), decoded.last()) else {
                return Vec::new();
            };
            edits.push(ItemEdit {
                item,
                local: first.start..last.end,
            });
        }
        edits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_back_rewrites_only_the_changed_stretch_widened_to_whole_references() {
        let raw = "a&amp;b&amp;c";
        let span = MarkupSpan::escaped(raw, 0..raw.len(), SpanKind::Text, CharacterReferences::Xml);
        // "a&b&c" → "a&X&c": the middle `b` alone.
        assert_eq!(span.write_back("a&X&c"), Some((6..7, "X".to_owned())));
        // "a&b&c" → "a?b&c": the first reference, whole.
        assert_eq!(span.write_back("a?b&c"), Some((1..6, "?".to_owned())));
        // An insertion lands on the character after it.
        assert_eq!(span.write_back("a&b&cd"), Some((12..13, "cd".to_owned())));
        assert_eq!(span.write_back("a&b&c"), None);
    }
}
