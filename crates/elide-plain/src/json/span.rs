//! [`JsonSpan`]: where a JSON key, string value, or scalar was read from and
//! how a new value is written back, and the [`JsonAddresser`] that maps a
//! decoded range to and from the raw bytes through it.

use std::ops::Range;

use elide_codec::extract::{ExtractedItem, ItemEdit, OffsetMap, Rewrite, SourceAddresser};
use elide_core::modality::text::SourceRef;

use super::escape::json_escape;
use super::parser::is_json_literal;

/// A JSON [`ExtractedItem`]: a key, string value, or scalar, addressed by the
/// source bytes its value was read from.
pub(super) type JsonItem = ExtractedItem<JsonSpan>;

/// Where a JSON value was read from in the source, and how a new value is
/// written back there.
///
/// The value is the text a reader sees: a string's escapes (`\"`, `\u00e9`)
/// are decoded, and [`offsets`](Self::offsets) maps it back onto the raw
/// bytes, each escape atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct JsonSpan {
    /// The raw bytes the value was read from: a string's between its quotes,
    /// a scalar's whole literal.
    pub(super) range: Range<usize>,
    /// The map from the value back onto [`range`](Self::range).
    pub(super) offsets: OffsetMap,
    /// Whether the value is a quoted string (a key or a string value) rather
    /// than a bare scalar.
    pub(super) quoted: bool,
    /// The value as read, which a write-back compares the current value
    /// against.
    pub(super) read: String,
}

impl JsonSpan {
    /// The whole token in the source: a string's quotes included.
    fn token(&self) -> Range<usize> {
        if self.quoted {
            self.range.start - 1..self.range.end + 1
        } else {
            self.range.clone()
        }
    }

    /// The raw range to replace, and its new raw text, that write `value` back
    /// over this span; `None` while `value` is as read.
    ///
    /// A string rewrites only the stretch a redaction changed, escaped, so an
    /// escape outside it keeps its spelling. A scalar is rewritten whole: bare
    /// while it is still a JSON literal, and as a string once it is not
    /// (masking `42` with `XXX` must not leave a bare `XXX`).
    pub(super) fn write_back(&self, value: &str) -> Option<Splice> {
        if !self.quoted {
            let text = if is_json_literal(value) {
                value.to_owned()
            } else {
                format!("\"{}\"", json_escape(value))
            };
            return (value != self.read).then(|| Splice {
                raw: self.range.clone(),
                text,
            });
        }
        let Rewrite { raw, text } = self.offsets.rewrite(&self.read, value)?;
        Some(Splice {
            raw,
            text: json_escape(text),
        })
    }
}

/// A rewrite of the JSON source: the raw range replaced, and its new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Splice {
    /// The raw range replaced.
    pub(super) raw: Range<usize>,
    /// The raw text written there.
    pub(super) text: String,
}

/// Maps a JSON item's decoded value range to and from its raw source bytes
/// through its [`JsonSpan`]'s offset map. JSON is a single file, so a
/// part-tagged reference addresses nothing.
#[derive(Debug, Default)]
pub(super) struct JsonAddresser;

impl SourceAddresser<JsonSpan> for JsonAddresser {
    fn source_span(&self, item: &JsonItem, local: Range<usize>) -> Vec<SourceRef> {
        item.address
            .offsets
            .raw_ranges(local)
            .into_iter()
            .map(SourceRef::new)
            .collect()
    }

    /// Every reference must address one item, or none resolves.
    fn locate_source(&self, items: &[JsonItem], source: &[SourceRef]) -> Vec<ItemEdit> {
        source
            .iter()
            .map(|reference| locate(items, reference))
            .collect::<Option<_>>()
            .unwrap_or_default()
    }
}

/// The edit `reference` addresses: the part of one item's value its raw bytes
/// decode to.
///
/// The reference must lie within one token, and may take a string's quotes
/// with what they enclose (the whole `"…"` addresses the whole value), but
/// none addresses a quote alone, nothing, or half an escape, which have no
/// value of their own to redact.
fn locate(items: &[JsonItem], reference: &SourceRef) -> Option<ItemEdit> {
    let raw = &reference.range;
    if reference.part.is_some() || raw.start >= raw.end {
        return None;
    }
    // Tokens are disjoint and in source order.
    let item = items.partition_point(|item| item.address.token().end <= raw.start);
    let span = &items.get(item)?.address;
    let token = span.token();
    if raw.start < token.start || token.end < raw.end {
        return None;
    }
    let inner = raw.start.max(span.range.start)..raw.end.min(span.range.end);
    if inner.is_empty() {
        return None;
    }
    let local = hull(&span.offsets.decoded_ranges(inner.clone()))?;
    (hull(&span.offsets.raw_ranges(local.clone()))? == inner).then_some(ItemEdit { item, local })
}

/// The range from the first of `ranges` to the last.
fn hull(ranges: &[Range<usize>]) -> Option<Range<usize>> {
    Some(ranges.first()?.start..ranges.last()?.end)
}
