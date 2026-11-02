//! [`XmlSpan`]: where a markup item's value was read from and how a new value
//! is written back, and the [`MarkupAddresser`] that maps a decoded range to
//! and from the raw bytes through it.

use std::ops::Range;

use elide_codec::extract::{ExtractedItem, ItemEdit, OffsetMap, Passage, SourceAddresser};
use elide_core::modality::text::SourceRef;
use quick_xml::escape::{escape, partial_escape};

use crate::references::{CharacterReferences, decode_references};

/// An XML [`ExtractedItem`] addressed by the source bytes its `value` was read
/// from.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
pub(crate) type XmlItem = ExtractedItem<XmlSpan>;

/// Where an [`ExtractedItem`]'s value was read from in the retained raw
/// document, and how a new value is written back there.
///
/// The value is the text a reader sees: a text run's or an attribute value's
/// character references (`&amp;`, `&#64;`, HTML's `&nbsp;`) are decoded, and
/// [`offsets`](Self::offsets) maps it back onto the raw bytes.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlSpan {
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

/// How a new value is escaped where an [`XmlSpan`] lands.
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

impl XmlSpan {
    /// The value of the escaped `kind` text at `range` of `raw`, its
    /// references decoded by `references`, and its span.
    pub(crate) fn escaped(
        raw: &str,
        range: Range<usize>,
        kind: SpanKind,
        references: CharacterReferences,
    ) -> (String, Self) {
        let (value, offsets) = decode_references(&raw[range.clone()], range.start, references);
        (value.clone(), Self::new(range, offsets, kind, value))
    }

    /// The value of the verbatim text at `range` of `raw`, and its span.
    pub(crate) fn verbatim(raw: &str, range: Range<usize>) -> (String, Self) {
        let value = raw[range.clone()].to_owned();
        let offsets = OffsetMap::identity(range.start, range.len());
        (
            value.clone(),
            Self::new(range, offsets, SpanKind::Verbatim, value),
        )
    }

    /// The span of a separator reading as `value`, which stands for the markup
    /// at `range` and maps to no source text.
    pub(crate) fn separator(range: Range<usize>, value: &str) -> (String, Self) {
        let span = Self::new(
            range,
            OffsetMap::default(),
            SpanKind::Verbatim,
            value.to_owned(),
        );
        (value.to_owned(), span)
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
        let read = self.read.as_str();
        if value == read {
            return None;
        }
        let prefix = common_prefix(read, value);
        let suffix = common_suffix(&read[prefix..], &value[prefix..]);
        let mut changed = prefix..read.len() - suffix;
        // A pure insertion covers no read text; widen it by one character so it
        // lands on raw bytes.
        if changed.is_empty() {
            match read[changed.end..].chars().next() {
                Some(next) => changed.end += next.len_utf8(),
                None => changed.start -= read[..changed.start].chars().next_back()?.len_utf8(),
            }
        }
        let raw = self.offsets.raw_ranges(changed);
        let raw = raw.first()?.start..raw.last()?.end;
        let decoded = self.offsets.decoded_ranges(raw.clone());
        let decoded = decoded.first()?.start..decoded.last()?.end;
        let text = &value[decoded.start..value.len() - (read.len() - decoded.end)];
        let text = match self.kind {
            SpanKind::Text => partial_escape(text),
            SpanKind::Attribute => escape(text),
            SpanKind::Verbatim => text.into(),
        };
        Some((raw, text.into_owned()))
    }
}

/// The byte length of the longest common prefix of `a` and `b`, on a character
/// boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((at, _), _)| at)
}

/// The byte length of the longest common suffix of `a` and `b`, on a character
/// boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

/// Maps a markup item's decoded value range to/from its raw source bytes
/// through its [`XmlSpan`]'s offset map, single-file (no part tag).
#[derive(Debug, Default)]
pub(crate) struct MarkupAddresser;

impl SourceAddresser<XmlSpan> for MarkupAddresser {
    fn source_span(&self, item: &XmlItem, local: Range<usize>) -> Vec<SourceRef> {
        item.address
            .offsets
            .raw_ranges(local)
            .into_iter()
            .map(SourceRef::new)
            .collect()
    }

    fn locate_source(&self, items: &[XmlItem], source: &[SourceRef]) -> Vec<ItemEdit> {
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
        ItemEdit::merged(edits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_back_rewrites_only_the_changed_stretch_widened_to_whole_references() {
        let raw = "a&amp;b&amp;c";
        let (_, span) =
            XmlSpan::escaped(raw, 0..raw.len(), SpanKind::Text, CharacterReferences::Xml);
        // "a&b&c" → "a&X&c": the middle `b` alone.
        assert_eq!(span.write_back("a&X&c"), Some((6..7, "X".to_owned())));
        // "a&b&c" → "a?b&c": the first reference, whole.
        assert_eq!(span.write_back("a?b&c"), Some((1..6, "?".to_owned())));
        // An insertion lands on the character after it.
        assert_eq!(span.write_back("a&b&cd"), Some((12..13, "cd".to_owned())));
        assert_eq!(span.write_back("a&b&c"), None);
    }
}
