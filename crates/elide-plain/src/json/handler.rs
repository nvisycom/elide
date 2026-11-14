//! JSON handler: a flat ordered sequence of source slots.
//!
//! The loader lexes the source once into [`Slot`]s: either
//! [`Slot::Passthrough`] (whitespace + structural punctuation, kept
//! verbatim) or [`Slot::Leaf`] (a key, string value, or scalar). Leaves
//! carry both the original source bytes (`serialized`) and the unescaped
//! UTF-8 value the recognizer sees (`value`). [`Stream::chunks`]
//! yields leaves in document order; `write_at` splices each redaction into
//! the leaf's `serialized` bytes at the mapped source span (keeping `value`
//! in sync); [`Encode::encode`] concatenates every slot.
//!
//! Because a redaction is spliced rather than re-rendered from the decoded
//! value, formatting (indentation, key order, whitespace) *and* every byte
//! outside a redacted span, including `\uXXXX` / `\"` escapes within a
//! redacted string, stay byte-identical to the source. The partial-leaf
//! offset translation is a single per-leaf walk of its escape table.

use std::ops::Range;

use elide_codec::content::ContentData;
use elide_codec::string::{ContiguousSpan, RedactRange};
use elide_codec::{Encode, Format, FormatId, Stream};
use elide_core::modality::text::{Marked, SourceRef, Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter, ResolvedHint};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::JsonLoader;
use super::escape::{decode_escape, json_escape};
use super::parser::parse_slots;
use crate::slice::slice;

/// Stable [`FormatId`] for the JSON codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.json");

/// The JSON [`Format`], resolved by `.json` and `application/json`.
pub fn format() -> Format {
    Format::new(FORMAT_ID, JsonLoader)
        .with_extensions(["json"])
        .with_content_types(["application/json"])
}

/// A resolved redaction: which leaf slot to edit, and the value-local byte
/// range within it that [`Leaf::splice`] replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LeafEdit {
    /// Index of the target [`Slot::Leaf`] in `slots`.
    slot: usize,
    /// The byte range to replace within that leaf's decoded value.
    value: Range<usize>,
}

/// The interior of a quoted leaf: the source byte range between its two `"`
/// delimiters, in document coordinates. The escape walk and the source↔value
/// maps all bound their cursor to `start..end`, and the delimiter check asks
/// whether a span misses the interior entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EscapedBounds {
    /// Source offset of the first interior byte (just past the opening `"`).
    start: usize,
    /// Source offset just past the last interior byte (the closing `"`).
    end: usize,
}

impl EscapedBounds {
    /// Whether a source `raw` span covers only a `"` delimiter and no interior
    /// byte, the opening quote alone (`raw.end <= start`) or the closing quote
    /// alone (`raw.start >= end`). Such a span maps to a zero-width edit at a
    /// value boundary, which `splice` would *insert* rather than redact.
    fn is_delimiter_only(&self, raw: &Range<usize>) -> bool {
        raw.end <= self.start || raw.start >= self.end
    }
}

/// A leaf located within the slot stream: its slot index, a borrow of the leaf,
/// and where its value starts in the decoded stream, enough to lift a decoded
/// offset into and out of it without re-sweeping the slots.
struct LeafAt<'a> {
    /// Index of the leaf's [`Slot`] in `slots`.
    index: usize,
    /// The located leaf.
    leaf: &'a Leaf,
    /// The leaf value's start offset in the decoded stream.
    decoded_start: usize,
}

/// One element of the parsed source.
#[derive(Debug, Clone)]
pub(super) enum Slot {
    /// Whitespace or structural punctuation (`{ } [ ] : ,` and
    /// surrounding whitespace). Held verbatim and emitted back unchanged.
    Passthrough(String),
    /// A key, string value, or scalar (number/bool/null): every position
    /// a recognizer is allowed to address.
    Leaf(Leaf),
}

/// An addressable position in the document.
#[derive(Debug, Clone)]
pub(super) struct Leaf {
    pub kind: LeafKind,
    /// Current unescaped UTF-8 value: what the recognizer sees in
    /// [`Chunk::data`] and what redactions edit.
    pub value: String,
    /// Current source bytes: what `encode` emits and what a raw
    /// [`SourceRef`] addresses. For [`LeafKind::Key`] and
    /// [`LeafKind::StringValue`] this is the quoted form `"…"` with `\\` /
    /// `\"` escapes; for [`LeafKind::Scalar`] it is the bare literal.
    ///
    /// [`TextLocation`] offsets address the *decoded* stream ([`value`](Self::value)),
    /// not these bytes, the two differ wherever an escape collapses.
    pub serialized: String,
    /// Out-of-band located context (currently the enclosing object key)
    /// surfaced to recognizers as hints; empty for keys and for value
    /// leaves outside any object (e.g. a top-level scalar). Each hint
    /// carries the key's source span so a boost can point back at the key.
    pub hints: Vec<ResolvedHint<Text>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeafKind {
    Key,
    StringValue,
    Scalar,
}

impl Leaf {
    fn is_quoted(&self) -> bool {
        matches!(self.kind, LeafKind::Key | LeafKind::StringValue)
    }

    /// Replace the decoded-value byte range `value_range` with `replacement`,
    /// editing `serialized` **in place** so every byte outside the edited span,
    /// quote, indentation, and any `\uXXXX` / `\"` escapes, survives
    /// verbatim, rather than re-rendering the whole value (which would flatten
    /// escapes to their literal chars). `value` is kept in sync so later reads
    /// and edits of the same leaf see the new content.
    fn splice(&mut self, value_range: Range<usize>, replacement: &str) -> Result<()> {
        if !self.is_quoted() {
            // A scalar has no escapes and no quotes; value and serialized are
            // the same bytes, so a direct splice of both keeps them in step.
            self.value.redact_range(replacement, value_range.clone())?;
            self.serialized.redact_range(replacement, value_range)?;
            // A spliced scalar may no longer be a valid JSON literal (masking
            // `42` with `XXX` yields bare `XXX`). Promote such a value to a
            // quoted string so the document stays valid JSON.
            if !is_json_literal(&self.serialized) {
                self.serialized = format!("\"{}\"", json_escape(&self.value));
                self.kind = LeafKind::StringValue;
            }
            return Ok(());
        }
        // Map the value range to source offsets within `serialized` (offset 0 =
        // the leaf's own bytes), then splice the escaped replacement there.
        let source_start = self
            .value_to_source(0, value_range.start)
            .ok_or_else(|| Error::malformed("value offset out of range"))?;
        let source_end = self
            .value_to_source(0, value_range.end)
            .ok_or_else(|| Error::malformed("value offset out of range"))?;
        self.serialized
            .redact_range(&json_escape(replacement), source_start..source_end)?;
        self.value.redact_range(replacement, value_range)?;
        Ok(())
    }

    /// Map a byte offset in this leaf's decoded [`value`](Self::value) to the
    /// source byte offset it sits at in [`serialized`](Self::serialized), the
    /// escape-aware forward mapping. `slot_start` is the leaf's source byte
    /// offset in the document (0 when addressing the leaf's own bytes).
    ///
    /// For a scalar the mapping is identity. For a quoted leaf the value cursor
    /// advances one per interior source byte (or by an escape's decoded width
    /// across a `\` escape). `None` if `value_offset` is past the value or does
    /// not land on a decoded-character boundary (splitting a multi-byte char an
    /// escape produced would map to a source offset wider than the request).
    fn value_to_source(&self, slot_start: usize, value_offset: usize) -> Option<usize> {
        if !self.is_quoted() {
            if value_offset > self.value.len() {
                return None;
            }
            return Some(slot_start + value_offset);
        }
        let bounds = self.escaped_bounds(slot_start);
        let bytes = self.serialized.as_bytes();
        let mut src = bounds.start;
        let mut val = 0usize;
        while val < value_offset {
            if src >= bounds.end {
                return None;
            }
            let width = self.token_width(bytes, src - slot_start)?;
            src += width.source;
            val += width.value;
        }
        // `val` overshoots when `value_offset` fell inside a multi-byte decoded
        // char; only an exact boundary has a well-defined source offset.
        (val == value_offset).then_some(src)
    }

    /// Inverse of [`value_to_source`](Self::value_to_source): map a source byte
    /// offset in this leaf's [`serialized`](Self::serialized) bytes to the value
    /// byte offset it decodes to.
    ///
    /// The offset must land on an escape boundary (never mid-`\uXXXX`). The
    /// quotes bound the value: the opening quote maps to value offset 0, the
    /// closing quote to the value's end, so a span covering the whole `"…"`
    /// token resolves to the whole value. `None` if `source_offset` is outside
    /// the leaf or splits an escape.
    fn source_to_value(&self, slot_start: usize, source_offset: usize) -> Option<usize> {
        if !self.is_quoted() {
            let end = slot_start + self.serialized.len();
            return (slot_start..=end)
                .contains(&source_offset)
                .then_some(source_offset - slot_start);
        }
        let bounds = self.escaped_bounds(slot_start);
        // The quotes bound the value: the opening quote (`slot_start`) maps to 0;
        // the closing quote (`bounds.end`) and the token's exclusive end (one
        // past it) both map to the value's end, so a span covering the whole
        // `"…"` token resolves to the whole value.
        if source_offset == slot_start {
            return Some(0);
        }
        if source_offset == bounds.end || source_offset == bounds.end + 1 {
            return Some(self.value.len());
        }
        if source_offset < bounds.start || source_offset > bounds.end {
            return None;
        }
        let bytes = self.serialized.as_bytes();
        let mut src = bounds.start;
        let mut val = 0usize;
        while src < source_offset {
            let width = self.token_width(bytes, src - slot_start)?;
            src += width.source;
            val += width.value;
        }
        // Landed inside an escape rather than on a boundary.
        (src == source_offset).then_some(val)
    }

    /// The [`EscapedBounds`] of this quoted leaf: the source byte range of its
    /// interior, between the two `"`, in document coordinates. `slot_start` is
    /// the leaf's own source offset in the document.
    fn escaped_bounds(&self, slot_start: usize) -> EscapedBounds {
        EscapedBounds {
            start: slot_start + 1,
            end: slot_start + self.serialized.len() - 1,
        }
    }

    /// The [`TokenWidth`] of the token at byte `local` in the leaf's serialized
    /// bytes: for a `\` escape its source span and decoded width; for a literal
    /// character its UTF-8 width (equal in both dimensions, so a multi-byte
    /// literal like `é` is stepped over whole and an offset can never land inside
    /// it). `None` for a malformed escape or a non-boundary `local`.
    fn token_width(&self, bytes: &[u8], local: usize) -> Option<TokenWidth> {
        if bytes.get(local) == Some(&b'\\') {
            let (source_len, decoded) = decode_escape(&bytes[local..])?;
            Some(TokenWidth {
                source: source_len,
                value: decoded.len_utf8(),
            })
        } else {
            // A literal character occupies the same bytes in source and value.
            let width = self.serialized.get(local..)?.chars().next()?.len_utf8();
            Some(TokenWidth {
                source: width,
                value: width,
            })
        }
    }
}

/// How wide one serialized token is in each of the leaf's two coordinate spaces:
/// its source bytes (`serialized`) and the decoded bytes it produces (`value`).
/// Equal for a literal char; wider in `source` for a `\` escape (`é` is six
/// source bytes, two value bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TokenWidth {
    /// Bytes the token spans in the serialized source.
    source: usize,
    /// Bytes the token decodes to in the value.
    value: usize,
}

/// Whether `s` is a bare JSON literal, a number, or one of `true` / `false`
/// / `null`. The lexer uses it to reject a malformed scalar, and the redactor
/// to decide whether a redacted scalar can stay unquoted or must be promoted
/// to a JSON string to keep the document valid.
///
/// The number test follows the JSON grammar (RFC 8259), *not* Rust's
/// `f64::from_str`, which is looser: it would accept `5.`, `007`, `-inf`, and
/// `NaN`, none of which are valid JSON. Accepting them here would let the lexer
/// pass malformed input and let a redaction emit a bare token that breaks the
/// document.
pub(super) fn is_json_literal(s: &str) -> bool {
    matches!(s, "true" | "false" | "null") || is_json_number(s)
}

/// Whether `s` matches the JSON number grammar:
/// `-?(0 | [1-9][0-9]*) (\.[0-9]+)? ([eE][+-]?[0-9]+)?`, an optional minus, an
/// integer part with no leading zeros, an optional fraction with at least one
/// digit, and an optional exponent.
fn is_json_number(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    let len = bytes.len();

    // Optional leading minus.
    if bytes.first() == Some(&b'-') {
        i = 1;
    }
    // Integer part: a lone `0`, or a non-zero digit followed by more digits.
    let int_start = i;
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            while matches!(bytes.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
        }
        _ => return false,
    }
    // A leading zero may not be followed by another digit (`007` is invalid).
    if bytes[int_start] == b'0' && matches!(bytes.get(i), Some(b'0'..=b'9')) {
        return false;
    }
    // Optional fraction: a dot followed by at least one digit.
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let frac_start = i;
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == frac_start {
            return false;
        }
    }
    // Optional exponent: e/E, optional sign, at least one digit.
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_start = i;
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == len
}

/// Handler for loaded JSON content: the slots and where each starts in both
/// coordinate spaces.
#[derive(Debug)]
pub(crate) struct JsonHandler {
    slots: Vec<Slot>,
    /// Each slot's start offsets, then the document's end: `offsets[i]` and
    /// `offsets[i + 1]` bound slot `i`. Rebuilt after a write, which changes
    /// leaf lengths.
    offsets: Vec<SlotOffset>,
}

/// Where a slot starts in the decoded stream (what [`TextLocation`] ranges
/// address) and in the serialized source (what a [`SourceRef`] addresses).
#[derive(Debug, Clone, Copy, Default)]
struct SlotOffset {
    decoded: usize,
    source: usize,
}

impl Stream<Text> for JsonHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        // A chunk's location addresses the decoded stream and matches `data`
        // (the decoded value), so a recognizer's offsets into `data` map
        // straight through. Only leaf slots yield a chunk.
        Ok(self
            .slots
            .iter()
            .zip(&self.offsets)
            .filter_map(|(slot, offset)| {
                let Slot::Leaf(leaf) = slot else {
                    return None;
                };
                let start = offset.decoded;
                Some(Chunk {
                    location: TextLocation::inline(start..start + leaf.value.len()),
                    data: TextData::new(leaf.value.clone()),
                    hints: leaf.hints.clone(),
                })
            })
            .collect())
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // `local` is a byte range into this leaf's *decoded* value. The lifted
        // location's span addresses the decoded stream (in step with the
        // recognizer's own coordinates); its source ref carries the raw span,
        // since a JSON value differs from its source wherever an escape (`\"`,
        // `\uXXXX`) collapses.
        let LeafAt {
            index,
            leaf,
            decoded_start: decoded_slot_start,
        } = self.find_leaf(chunk.location.range()?)?;
        let local_range = local.range()?;
        let decoded_start = decoded_slot_start + local_range.start;
        let decoded_end = decoded_slot_start + local_range.end;

        let source_slot_start = self.offsets[index].source;
        let source_start = leaf.value_to_source(source_slot_start, local_range.start)?;
        let source_end = leaf.value_to_source(source_slot_start, local_range.end)?;

        Some(TextLocation::source(
            decoded_start..decoded_end,
            [SourceRef::new(source_start..source_end)],
        ))
    }

    /// A source-byte mark becomes the decoded span of the value it covers,
    /// keeping its source references.
    fn resolve(&self, location: TextLocation) -> Result<TextLocation> {
        let TextLocation::Marked(mark) = &location else {
            return Ok(location);
        };
        let Marked::Source { refs } = mark else {
            return Err(Error::malformed(
                "a rendered-page mark does not address JSON",
            ));
        };
        let (first, rest) = refs
            .split_first()
            .expect("a mark has at least one source reference");
        let edit = self.resolve_source(first, rest)?;
        let start = self.offsets[edit.slot].decoded;
        Ok(TextLocation::source(
            start + edit.value.start..start + edit.value.end,
            refs.clone(),
        ))
    }
}

impl Encode for JsonHandler {
    fn encode(&self) -> Result<ContentData> {
        let mut out = String::new();
        for slot in &self.slots {
            match slot {
                Slot::Passthrough(text) => out.push_str(text),
                Slot::Leaf(leaf) => out.push_str(&leaf.serialized),
            }
        }
        Ok(ContentData::from_text(out))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for JsonHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<TextData> {
        let range = location
            .range()
            .ok_or_else(|| Error::malformed("an unresolved mark has no span"))?;
        let found = self.find_leaf(range).ok_or_else(|| {
            Error::malformed(format!(
                "range {}..{} is not within one JSON value",
                range.start, range.end
            ))
        })?;
        let local = range.start - found.decoded_start..range.end - found.decoded_start;
        slice(&found.leaf.value, &local).map(TextData::new)
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for JsonHandler {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        // Resolve every redaction to a `(slot, value range)` plan against the
        // **pre-mutation** slots first: mutating a leaf shifts later slots'
        // offsets, but a per-leaf value range stays valid regardless. Any
        // location that resolves to no value is an error.
        let mut plan: Vec<(LeafEdit, String)> = Vec::new();
        for (loc, replacement) in redactions.into_iter() {
            let edit = self.edit_for(&loc)?;
            let value = replacement.value().unwrap_or_default().to_owned();
            plan.push((edit, value));
        }
        // Apply per-leaf edits right-to-left within each leaf so earlier
        // edits in the same leaf don't shift later ones.
        plan.sort_by(|(a, _), (b, _)| a.slot.cmp(&b.slot).then(b.value.start.cmp(&a.value.start)));
        let edited = !plan.is_empty();
        for (edit, value) in plan {
            let Slot::Leaf(leaf) = &mut self.slots[edit.slot] else {
                continue;
            };
            leaf.splice(edit.value, &value)?;
        }
        if edited {
            self.offsets = offsets(&self.slots);
        }
        Ok(())
    }
}

/// The start offsets of every slot, then the document's end.
fn offsets(slots: &[Slot]) -> Vec<SlotOffset> {
    let mut offsets = Vec::with_capacity(slots.len() + 1);
    let mut at = SlotOffset::default();
    offsets.push(at);
    for slot in slots {
        at.decoded += slot.decoded_len();
        at.source += slot.source_len();
        offsets.push(at);
    }
    offsets
}

impl JsonHandler {
    /// Lex JSON source into a handler, preserving its formatting verbatim.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the source is not well-formed JSON.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn parse(source: &str) -> Result<Self> {
        let slots = parse_slots(source)?;
        let offsets = offsets(&slots);
        Ok(Self { slots, offsets })
    }

    /// The slots that can hold offset `at` in one coordinate space, in order:
    /// those starting at or before it and ending at or after it. More than
    /// one only where empty slots (an empty string's value) sit at `at`.
    fn slots_at(&self, at: usize, coordinate: fn(&SlotOffset) -> usize) -> Range<usize> {
        let first = self.offsets[1..].partition_point(|end| coordinate(end) < at);
        let last =
            self.offsets[..self.slots.len()].partition_point(|start| coordinate(start) <= at);
        first..last.max(first)
    }

    /// Locate the leaf whose *decoded* range contains `range`, with its index
    /// and its decoded-stream start offset.
    fn find_leaf(&self, range: &Range<usize>) -> Option<LeafAt<'_>> {
        self.slots_at(range.start, |o| o.decoded).find_map(|index| {
            let Slot::Leaf(leaf) = &self.slots[index] else {
                return None;
            };
            (range.end <= self.offsets[index + 1].decoded).then(|| LeafAt {
                index,
                leaf,
                decoded_start: self.offsets[index].decoded,
            })
        })
    }

    /// The [`LeafEdit`] a redaction `location` targets.
    ///
    /// A location carrying a raw [`source`](TextLocation::source) reference is
    /// resolved through the serialized bytes (JSON is single-file, so any
    /// `part`-tagged ref is rejected); otherwise the decoded
    /// [`range`](TextLocation::range) locates the leaf. Either way the returned
    /// value range feeds the same splice.
    ///
    /// Every location that resolves to no value is `Err`: dropping the edit
    /// would leave a green audit over an unredacted document. That covers a
    /// location with neither a range nor a source ref, and a malformed range
    /// (reversed, structural, past-value, escape-splitting).
    ///
    /// A location's [`source`](TextLocation::source) may hold **several** refs,
    /// one logical span fragmented across escapes or fused from several runs.
    /// *Every* ref is resolved, *all* must land in the same leaf, and only
    /// whitespace may separate them: the edit covers them together. Resolving
    /// only the first would splice
    /// it and report success while ignoring the rest, leaving the caller's later
    /// coordinates unredacted under a green audit.
    fn edit_for(&self, location: &TextLocation) -> Result<LeafEdit> {
        match location.source_refs().split_first() {
            Some((first, rest)) => self.resolve_source(first, rest),
            None => self.resolve_range(location),
        }
    }

    /// Resolve a location's raw [`SourceRef`] list to a single [`LeafEdit`].
    /// Each raw byte span is mapped to the leaf's value coordinate through the
    /// escape table; every ref must resolve to the *same* leaf, and the value
    /// ranges must form one span (see [`ContiguousSpan`]), which the edit
    /// covers.
    fn resolve_source(&self, first: &SourceRef, rest: &[SourceRef]) -> Result<LeafEdit> {
        let edit = self.resolve_one_source(first)?;
        let mut pieces = vec![edit.value];
        for source in rest {
            let next = self.resolve_one_source(source)?;
            if next.slot != edit.slot {
                return Err(Error::malformed(format!(
                    "source references span more than one JSON value (slots {} and {})",
                    edit.slot, next.slot
                )));
            }
            pieces.push(next.value);
        }
        let Slot::Leaf(leaf) = &self.slots[edit.slot] else {
            unreachable!("a source reference resolves to a leaf");
        };
        Ok(LeafEdit {
            slot: edit.slot,
            value: leaf.value.contiguous_span(pieces)?,
        })
    }

    /// Resolve one raw [`SourceRef`] to the [`LeafEdit`] for its leaf. An
    /// unresolvable or malformed ref is an error, never a silent no-op.
    fn resolve_one_source(&self, source: &SourceRef) -> Result<LeafEdit> {
        if let Some(part) = &source.part {
            return Err(Error::malformed(format!(
                "source reference names container part {part:?}, but JSON has no parts"
            )));
        }
        let raw = &source.range;
        if raw.start > raw.end {
            return Err(Error::malformed(format!(
                "source range {}..{} is reversed",
                raw.start, raw.end
            )));
        }
        // An empty span addresses no content to redact. Its endpoints map to a
        // single value offset, so the edit would be zero-width and `Leaf::splice`
        // would *insert* the replacement there rather than redact, for a quoted
        // interior (`2..2` in `"bcd"`) and for a scalar alike, whose mapping is
        // identity. Reject it before any mapping.
        if raw.start == raw.end {
            return Err(Error::malformed(format!(
                "source range {}..{} is empty",
                raw.start, raw.end
            )));
        }
        for idx in self.slots_at(raw.start, |o| o.source) {
            let Slot::Leaf(leaf) = &self.slots[idx] else {
                continue;
            };
            if raw.end > self.offsets[idx + 1].source {
                continue;
            }
            let slot_start = self.offsets[idx].source;
            // A quoted leaf's span may touch its `"` delimiters (the whole
            // `"…"` token resolves to the whole value), but a span that covers
            // *only* a quote addresses no redactable content: its endpoints
            // would both collapse to the same value boundary, and `splice`
            // would then *insert* the replacement into the adjacent value.
            if leaf.is_quoted() && leaf.escaped_bounds(slot_start).is_delimiter_only(raw) {
                return Err(Error::malformed(format!(
                    "source range {}..{} covers only a JSON string delimiter, no value",
                    raw.start, raw.end
                )));
            }
            let split = || {
                Error::malformed(format!(
                    "source range {}..{} splits a JSON escape",
                    raw.start, raw.end
                ))
            };
            let value_start = leaf
                .source_to_value(slot_start, raw.start)
                .ok_or_else(split)?;
            let value_end = leaf
                .source_to_value(slot_start, raw.end)
                .ok_or_else(split)?;
            return Ok(LeafEdit {
                slot: idx,
                value: value_start..value_end,
            });
        }
        // The span fell in structural bytes (a `{`, `,`, whitespace) or past the
        // document: no leaf value to redact.
        Err(Error::malformed(format!(
            "source range {}..{} resolves to no JSON string value",
            raw.start, raw.end
        )))
    }

    /// Resolve a decoded [`range`] to a [`LeafEdit`]. The decoded stream *is* the
    /// leaf's value within a leaf, so the value offsets are the decoded-local
    /// ones directly. Errors name every malformed shape: missing, reversed,
    /// structural start, past-value end, char-splitting, past-document.
    ///
    /// [`range`]: TextLocation::range
    fn resolve_range(&self, location: &TextLocation) -> Result<LeafEdit> {
        let range = location.range().ok_or_else(|| {
            Error::malformed("a JSON redaction needs a byte range or a source reference")
        })?;
        if range.start > range.end {
            return Err(Error::malformed(format!(
                "redaction range {}..{} is reversed",
                range.start, range.end
            )));
        }
        // The first slot ending past `range.start`: an empty slot there holds
        // nothing to redact.
        let idx = self.offsets[1..].partition_point(|end| end.decoded <= range.start);
        let Some(slot) = self.slots.get(idx) else {
            return Err(Error::malformed(format!(
                "redaction range {}..{} is past the end of the document",
                range.start, range.end
            )));
        };
        let Slot::Leaf(leaf) = slot else {
            return Err(Error::malformed(format!(
                "redaction range {}..{} does not start in a JSON value",
                range.start, range.end
            )));
        };
        let slot_start = self.offsets[idx].decoded;
        let value = range.start - slot_start..range.end - slot_start;
        if value.end > leaf.value.len() {
            return Err(Error::malformed(format!(
                "redaction range {}..{} extends past its JSON value",
                range.start, range.end
            )));
        }
        slice(&leaf.value, &value)?;
        Ok(LeafEdit { slot: idx, value })
    }
}

impl Slot {
    /// This slot's length in the decoded stream, the decoded value for a leaf,
    /// the verbatim bytes for passthrough.
    fn decoded_len(&self) -> usize {
        match self {
            Slot::Passthrough(t) => t.len(),
            Slot::Leaf(l) => l.value.len(),
        }
    }

    /// This slot's length in the serialized stream, the escaped source bytes
    /// for a leaf, the verbatim bytes for passthrough.
    fn source_len(&self) -> usize {
        match self {
            Slot::Passthrough(t) => t.len(),
            Slot::Leaf(l) => l.serialized.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::modality::text::TextReplacement;

    use super::*;

    fn handler(src: &str) -> JsonHandler {
        JsonHandler::parse(src).expect("valid JSON")
    }

    /// The first chunk whose decoded value equals `value`.
    fn find_chunk(h: &JsonHandler, value: &str) -> Chunk<Text> {
        h.chunks()
            .unwrap()
            .into_iter()
            .find(|c| c.data.as_str() == value)
            .expect("chunk")
    }

    #[test]
    fn is_json_literal_follows_the_json_number_grammar() {
        // Valid: keywords, integers, a lone zero, signed, fraction, exponent.
        for ok in [
            "true", "false", "null", "0", "-0", "42", "-3.14", "1e10", "1E+10", "2.5e-3", "0.5",
        ] {
            assert!(is_json_literal(ok), "{ok:?} should be a valid literal");
        }
        // Invalid: Rust's f64 parser accepts these, JSON does not.
        for bad in [
            "5.", "007", "-inf", "inf", "NaN", "Infinity", "+5", ".5", "1.", "0x1f", "1.2.3", "",
            "01",
        ] {
            assert!(!is_json_literal(bad), "{bad:?} must be rejected");
        }
    }

    fn encoded(h: &JsonHandler) -> String {
        h.encode().unwrap().decode().unwrap()
    }

    #[tokio::test]
    async fn stream_yields_keys_and_values_in_order() -> Result<()> {
        let h = handler(r#"{"name":"Alice","age":30}"#);
        let chunks: Vec<String> = h
            .chunks()?
            .into_iter()
            .map(|c| c.data.as_str().to_owned())
            .collect();
        assert_eq!(chunks, vec!["name", "Alice", "age", "30"]);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_values_get_distinct_offsets() -> Result<()> {
        let h = handler(r#"{"a":"same","b":"same"}"#);
        let mut offsets = Vec::new();
        for c in h.chunks()? {
            if c.data.as_str() == "same" {
                offsets.push(c.location.range().unwrap().start);
            }
        }
        assert_eq!(offsets.len(), 2);
        assert_ne!(offsets[0], offsets[1]);
        Ok(())
    }

    /// A range inside a value reads that span, not the whole value: the
    /// anonymizer computes the replacement from what it reads.
    #[tokio::test]
    async fn read_returns_the_span_within_a_value() -> Result<()> {
        let h = handler(r#"{"note":"Contact: a@b.test"}"#);
        let chunk = find_chunk(&h, "Contact: a@b.test");
        let start = chunk.location.range().unwrap().start + "Contact: ".len();
        let email = TextLocation::inline(start..start + "a@b.test".len());
        assert_eq!(h.read_at(&email).await?.as_str(), "a@b.test");
        Ok(())
    }

    /// Offsets are rebuilt after a write that changes a value's length.
    #[tokio::test]
    async fn a_write_moves_the_later_chunks() -> Result<()> {
        let mut h = handler(r#"{"a":"long value","b":"next"}"#);
        let first = find_chunk(&h, "long value");
        h.write_at(Redactions::from([(
            first.location.clone(),
            TextReplacement::substituted("x"),
        )]))
        .await?;
        let next = find_chunk(&h, "next");
        assert_eq!(h.read_at(&next.location).await?.as_str(), "next");
        assert_eq!(encoded(&h), r#"{"a":"x","b":"next"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn read_returns_string() -> Result<()> {
        let h = handler(r#"{"name":"Alice"}"#);
        let mut found = false;
        for chunk in h.chunks()? {
            if h.read_at(&chunk.location).await?.as_str() == "Alice" {
                found = true;
            }
        }
        assert!(found);
        Ok(())
    }

    #[test]
    fn encode_preserves_source_compact() -> Result<()> {
        let src = r#"{"a":1}"#;
        assert_eq!(encoded(&handler(src)), src);
        Ok(())
    }

    #[test]
    fn encode_preserves_source_pretty() -> Result<()> {
        let src = "{\n  \"a\": 1\n}\n";
        assert_eq!(encoded(&handler(src)), src);
        Ok(())
    }

    #[tokio::test]
    async fn redact_whole_string_value() -> Result<()> {
        let mut h = handler(r#"{"name":"Alice"}"#);
        let chunk = find_chunk(&h, "Alice");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::substituted("Bob"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"name":"Bob"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_partial_leaf_in_compact_source() -> Result<()> {
        let src = r#"{"email":"alice@example.com"}"#;
        let mut h = handler(src);
        let chunk = find_chunk(&h, "alice@example.com");
        // Decoded-stream offsets: the chunk's decoded start plus the value-local
        // range of "alice" (offset 0 in the value here).
        let value = chunk.data.as_str();
        let at = value.find("alice").unwrap();
        h.write_at(Redactions::from([(
            TextLocation::inline(
                chunk.location.range().unwrap().start + at
                    ..chunk.location.range().unwrap().start + at + "alice".len(),
            ),
            TextReplacement::substituted("[USER]"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"email":"[USER]@example.com"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_partial_leaf_with_escapes() -> Result<()> {
        // decoded value: foo"bar; source: "foo\"bar", redacting "bar" in the
        // decoded stream splices back over the source bytes past the `\"` escape.
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = find_chunk(&h, r#"foo"bar"#);
        let value = chunk.data.as_str();
        let at = value.find("bar").unwrap();
        h.write_at(Redactions::from([(
            TextLocation::inline(
                chunk.location.range().unwrap().start + at
                    ..chunk.location.range().unwrap().start + at + "bar".len(),
            ),
            TextReplacement::substituted("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"msg":"foo\"XXX"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn u_escape_decodes_in_the_value_and_round_trips() -> Result<()> {
        // `é` is one `é` in the value but six source bytes; a
        // `😀` surrogate pair is one `😀` from twelve source bytes.
        // (Sources built at runtime so the literal backslash-u is unambiguous.)
        let e_acute = "\\u00e9";
        let grin = "\\uD83D\\uDE00";
        let src = format!("{{\"a\":\"caf{e_acute}\",\"b\":\"{grin}\"}}");
        let h = handler(&src);
        let values: Vec<String> = h
            .chunks()?
            .into_iter()
            .map(|c| c.data.as_str().to_owned())
            .collect();
        assert!(values.contains(&"caf\u{e9}".to_owned()), "got {values:?}");
        assert!(values.contains(&"\u{1F600}".to_owned()), "got {values:?}");
        // No redaction: the escapes are spliced back verbatim, never decoded.
        assert_eq!(encoded(&h), src);
        Ok(())
    }

    #[tokio::test]
    async fn redact_a_span_after_a_u_escape() -> Result<()> {
        let src = format!("{{\"msg\":\"caf{} bar\"}}", "\\u00e9");
        let mut h = handler(&src);
        let chunk = find_chunk(&h, "caf\u{e9} bar");
        // Decoded-stream offsets: in the value `café bar`, `caf` is bytes 0..3,
        // `é` is 3..5 (2 UTF-8 bytes), the space is 5, so `bar` starts at value
        // byte 6. The decoded location addresses it directly; write_at maps back
        // across the 6-byte `é` source escape when it splices.
        let value = chunk.data.as_str();
        let at = value.find("bar").unwrap();
        h.write_at(Redactions::from([(
            TextLocation::inline(
                chunk.location.range().unwrap().start + at
                    ..chunk.location.range().unwrap().start + at + "bar".len(),
            ),
            TextReplacement::substituted("XXX"),
        )]))
        .await?;
        // Only `bar` is replaced; the `é` escape before it is spliced through
        // verbatim (the redaction edits `serialized` in place, not a re-render of
        // the decoded value).
        assert_eq!(encoded(&h), format!("{{\"msg\":\"caf{} XXX\"}}", "\\u00e9"));
        Ok(())
    }

    #[tokio::test]
    async fn redacts_a_leaf_located_only_by_source() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A caller with only raw coordinates (no decoded range) locates the value
        // by its raw byte span in the serialized JSON. `bar` sits after the 6-byte
        // `é` escape, so its raw span differs from any decoded offset.
        let src = format!("{{\"msg\":\"caf{} bar\"}}", "\\u00e9");
        let mut h = handler(&src);
        let raw_at = src.find("bar").unwrap();
        let location = TextLocation::marked_source(SourceRef::new(raw_at..raw_at + "bar".len()));
        h.write_at(Redactions::from([(
            location,
            TextReplacement::substituted("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), format!("{{\"msg\":\"caf{} XXX\"}}", "\\u00e9"));
        Ok(())
    }

    /// A source-byte mark resolves to the decoded span of what it covers, so
    /// it reads like a detected entity.
    #[tokio::test]
    async fn a_mark_resolves_to_its_decoded_span() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        let src = format!("{{\"msg\":\"caf{} bar\"}}", "\\u00e9");
        let h = handler(&src);
        let raw_at = src.find("bar").unwrap();
        let mark = TextLocation::marked_source(SourceRef::new(raw_at..raw_at + 3));
        let resolved = h.resolve(mark)?;
        assert_eq!(h.read_at(&resolved).await?.as_str(), "bar");
        let structural = TextLocation::marked_source(SourceRef::new(0..1));
        assert!(h.resolve(structural).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn source_span_covering_the_whole_quoted_token_redacts_the_value() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A caller pointing at the *entire* `"bcd"` token, quotes included, must
        // resolve to the whole value (opening quote -> 0, closing -> value end),
        // not fail with a misleading "splits an escape".
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let open = src.find(r#""bcd""#).unwrap();
        let close = open + r#""bcd""#.len();
        let location = TextLocation::marked_source(SourceRef::new(open..close));
        h.write_at(Redactions::from([(
            location,
            TextReplacement::substituted("X"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"a":"X"}"#);
        Ok(())
    }

    #[test]
    fn source_to_value_rejects_an_offset_inside_a_literal_multibyte_char() {
        // A *literal* `é` (2 UTF-8 bytes, written directly, no `\u` escape).
        // A source offset between its two bytes splits the char and must be
        // rejected, not silently accepted as a value boundary.
        let h = handler("{\"a\":\"caf\u{e9}\"}");
        let Slot::Leaf(leaf) = h
            .slots
            .iter()
            .find(|s| matches!(s, Slot::Leaf(l) if l.value == "caf\u{e9}"))
            .unwrap()
        else {
            unreachable!()
        };
        // The leaf's serialized bytes are `"café"` at slot_start 0; `caf` is
        // 1..4 (after the opening quote), `é` is 4..6. An offset of 5 is inside
        // the literal `é`.
        assert!(leaf.source_to_value(0, 4).is_some(), "before é resolves");
        assert!(leaf.source_to_value(0, 6).is_some(), "after é resolves");
        assert_eq!(leaf.source_to_value(0, 5), None, "mid-é must be rejected");
    }

    #[test]
    fn value_to_source_rejects_a_mid_character_offset() {
        // In `café` (`é` decodes to the 2-byte `é` at value bytes 3..5),
        // offset 4 splits the char, there is no well-defined source offset, so
        // the mapping must reject it rather than overshoot past the escape.
        let h = handler(&format!("{{\"a\":\"caf{}\"}}", "\\u00e9"));
        let Slot::Leaf(leaf) = h
            .slots
            .iter()
            .find(|s| matches!(s, Slot::Leaf(l) if l.value == "caf\u{e9}"))
            .unwrap()
        else {
            unreachable!()
        };
        // Boundaries resolve; the mid-`é` offset (4) does not.
        assert!(leaf.value_to_source(0, 3).is_some());
        assert!(leaf.value_to_source(0, 5).is_some());
        assert_eq!(leaf.value_to_source(0, 4), None);
    }

    #[tokio::test]
    async fn a_part_tagged_source_ref_is_rejected_for_json() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // JSON is single-file; a part-tagged reference does not address it. The
        // caller supplied an explicit source coordinate to redact, so an
        // unresolvable one is an error, not a silent no-op that would leave a
        // green audit over an unredacted document.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let at = src.find("bcd").unwrap();
        let location =
            TextLocation::marked_source(SourceRef::in_part(at..at + 3, "some/part.json"));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("part-tagged ref must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_landing_in_structural_bytes_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // Offset 0 is the opening `{`, structural, no string value there. An
        // explicit source that resolves to no leaf must error, not silently
        // drop the edit.
        let mut h = handler(r#"{"a":"b"}"#);
        let location = TextLocation::marked_source(SourceRef::new(0..1));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("structural source must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_past_the_document_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A source span past every slot resolves to nothing, a caller mistake
        // (e.g. an off-by-one in a run→byte mapping), so it errors.
        let src = r#"{"a":"b"}"#;
        let mut h = handler(src);
        let past = src.len() + 4;
        let location = TextLocation::marked_source(SourceRef::new(past..past + 2));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("out-of-range source must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn every_source_ref_of_a_multi_ref_location_is_resolved() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A location whose source was fused from several runs carries several
        // refs. Both must resolve and the edit spans their union, resolving only
        // the first would leave the rest of the caller's span unredacted.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let location =
            TextLocation::marked_sources([SourceRef::new(b..b + 1), SourceRef::new(b + 1..b + 3)])
                .unwrap();
        h.write_at(Redactions::from([(
            location,
            TextReplacement::substituted("X"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"a":"X"}"#);
        Ok(())
    }

    /// Refs with text between them are refused: one edit over both would
    /// take that text too.
    #[tokio::test]
    async fn source_refs_with_text_between_them_are_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let location =
            TextLocation::marked_sources([SourceRef::new(b..b + 1), SourceRef::new(b + 2..b + 3)])
                .unwrap();
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("`c` lies between the refs");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src);
        Ok(())
    }

    #[tokio::test]
    async fn a_valid_first_ref_with_an_invalid_second_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // The first ref resolves cleanly; the second is out of range. The whole
        // location must be rejected, never splice the first and report success
        // while dropping the second, which would leave that coordinate unredacted
        // under a green audit.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let past = src.len() + 4;
        let location = TextLocation::marked_sources([
            SourceRef::new(b..b + 1),
            SourceRef::new(past..past + 2),
        ])
        .unwrap();
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("an invalid trailing ref must reject the whole location");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        // And nothing was spliced: the document is untouched.
        assert_eq!(encoded(&h), src, "no partial redaction on rejection");
        Ok(())
    }

    #[tokio::test]
    async fn source_refs_spanning_two_json_values_are_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // Two refs that each resolve, but to different leaves. Aggregating a span
        // across two values is meaningless, so it errors rather than silently
        // redacting just one.
        let src = r#"{"a":"bcd","e":"fgh"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let f = src.find("fgh").unwrap();
        let location =
            TextLocation::marked_sources([SourceRef::new(b..b + 1), SourceRef::new(f..f + 1)])
                .unwrap();
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("refs across two values must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "no partial redaction on rejection");
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_of_only_the_opening_quote_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A span covering just the opening `"` of `"bcd"` addresses no value
        // byte. Its endpoints would collapse to value offset 0 and `splice`
        // would insert `X` at the value start rather than redact, reject it.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let open = src.find(r#""bcd""#).unwrap();
        let location = TextLocation::marked_source(SourceRef::new(open..open + 1));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("a delimiter-only span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_of_only_the_closing_quote_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // The closing `"` of `"bcd"`. Its endpoints collapse to the value end,
        // where `splice` would append `X`, reject rather than corrupt.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let close = src.find(r#""bcd""#).unwrap() + r#""bcd""#.len() - 1;
        let location = TextLocation::marked_source(SourceRef::new(close..close + 1));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("a delimiter-only span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn an_empty_source_ref_inside_a_value_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A zero-width span in the interior of `"bcd"` (source `7..7`, at `c`).
        // Both endpoints map to one value offset, so `splice` would insert `X`
        // (yielding `bXcd`) rather than redact, reject it.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let c = src.find("bcd").unwrap() + 1;
        let location = TextLocation::marked_source(SourceRef::new(c..c));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("an empty interior span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn an_empty_source_ref_inside_a_scalar_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A scalar's source→value mapping is identity, so a zero-width span in it
        // has the same insert-not-redact gap. Reject it too.
        let src = r#"{"a":12345}"#;
        let mut h = handler(src);
        let mid = src.find("12345").unwrap() + 2;
        let location = TextLocation::marked_source(SourceRef::new(mid..mid));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("an empty scalar span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_range_starting_in_structural_json() -> Result<()> {
        // Offset 0 is the opening `{`, structural, not an addressable value.
        let mut h = handler(r#"{"a":"b"}"#);
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(0..1),
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_range_past_its_value() -> Result<()> {
        // A span that starts in the "b" value but runs past its end into the
        // closing quote / brace is rejected with a named error, not a late
        // generic splice failure.
        let mut h = handler(r#"{"a":"b"}"#);
        let chunk = find_chunk(&h, "b");
        let start = chunk.location.range().unwrap().start;
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(start..start + 5),
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_range_splitting_a_utf8_char() -> Result<()> {
        // In the value `café`, `é` is bytes 3..5; a range ending at 4 splits it.
        // The rejection must happen before any mutation, so the document is
        // left byte-identical.
        let src = format!("{{\"a\":\"caf{}\"}}", "\\u00e9");
        let mut h = handler(&src);
        let chunk = find_chunk(&h, "caf\u{e9}");
        let start = chunk.location.range().unwrap().start;
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(start + 3..start + 4),
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        // No partial mutation: the source is untouched after the rejected batch.
        assert_eq!(encoded(&h), src);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_reversed_range() -> Result<()> {
        // A reversed range (start > end) must be rejected before the value-local
        // `end - slot_offset` can underflow-panic in a checked build.
        let mut h = handler(r#"{"a":"bcd"}"#);
        let chunk = find_chunk(&h, "bcd");
        let start = chunk.location.range().unwrap().start;
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(start + 2..start),
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_range_at_the_document_end() -> Result<()> {
        // A start at (or past) the decoded-stream end resolves to no leaf; the
        // sweep must reject it rather than exhaust silently and return Ok.
        let mut h = handler(r#"{"a":"b"}"#);
        let decoded_len: usize = h.slots.iter().map(Slot::decoded_len).sum();
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(decoded_len..decoded_len + 1),
                TextReplacement::substituted("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn redact_key() -> Result<()> {
        let mut h = handler(r#"{"email":"a@b.c"}"#);
        let chunk = find_chunk(&h, "email");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::substituted("contact"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"contact":"a@b.c"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_scalar() -> Result<()> {
        let mut h = handler(r#"{"n":42}"#);
        let chunk = find_chunk(&h, "42");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::substituted("0"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"n":0}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_scalar_with_non_literal_replacement_quotes_it() -> Result<()> {
        // Masking a number with a non-literal string would make bare `XXX`
        // invalid JSON, so the leaf is promoted to a quoted string value.
        let mut h = handler(r#"{"n":42}"#);
        let chunk = find_chunk(&h, "42");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::substituted("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"n":"XXX"}"#);
        Ok(())
    }

    /// Multiple redactions in a single batch, each targeting a different
    /// leaf, with length deltas that shift later slot offsets. Regression
    /// test for the "only the first redaction lands" bug.
    #[tokio::test]
    async fn redact_multiple_leaves_with_shifting_offsets() -> Result<()> {
        let src = r#"{"a":"first","b":"second","c":"third"}"#;
        let mut h = handler(src);
        let mut locs = Vec::new();
        for c in h.chunks()? {
            let v = c.data.as_str();
            if v == "first" || v == "second" || v == "third" {
                locs.push(c.location);
            }
        }
        assert_eq!(locs.len(), 3, "expected three string values");
        let rs = locs
            .into_iter()
            .map(|loc| (loc, TextReplacement::substituted("X")))
            .collect();
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"a":"X","b":"X","c":"X"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn lift_simple_string() -> Result<()> {
        // No escapes: decoded and source coincide, so `range` and `.source`
        // agree, but `.source` is still populated with the raw span.
        let src = r#"{"email":"alice@example.com"}"#;
        let h = handler(src);
        let chunk = find_chunk(&h, "alice@example.com");
        let value_start = "alice@example.com".find("alice").unwrap();
        let value_end = value_start + "alice".len();
        let lifted = h
            .lift(&chunk, TextLocation::inline(value_start..value_end))
            .expect("range is in bounds");
        // Primary range: decoded-stream offset (chunk start + value-local).
        let lifted_range = lifted.range().unwrap();
        let chunk_start = chunk.location.range().unwrap().start;
        assert_eq!(lifted_range.start, chunk_start + value_start);
        assert_eq!(lifted_range.end, chunk_start + value_end);
        // Source: the raw byte span of "alice" in the document.
        let raw = src.find("alice").unwrap();
        assert_eq!(
            lifted.source_refs(),
            &[SourceRef::new(raw..raw + "alice".len())][..]
        );
        Ok(())
    }

    #[tokio::test]
    async fn lift_walks_escapes() -> Result<()> {
        // decoded value: foo"bar; source: "foo\"bar". The `\"` escape makes the
        // decoded stream one byte shorter than the source before "bar", so the
        // decoded `range` and the raw `.source` diverge, this is the case the
        // whole fix exists for.
        let src = r#"{"msg":"foo\"bar"}"#;
        let h = handler(src);
        let chunk = find_chunk(&h, r#"foo"bar"#);
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let lifted = h
            .lift(&chunk, TextLocation::inline(value_start..value_end))
            .expect("range is in bounds");
        // Decoded range: `bar` at value offset 4 (`foo"` = 4 decoded bytes).
        let lifted_range = lifted.range().unwrap();
        let chunk_start = chunk.location.range().unwrap().start;
        assert_eq!(lifted_range.start, chunk_start + value_start);
        assert_eq!(lifted_range.end, chunk_start + value_end);
        // Raw source span: `bar` sits after the 2-byte `\"` escape.
        let raw = src.find("bar").unwrap();
        assert_eq!(
            lifted.source_refs(),
            &[SourceRef::new(raw..raw + "bar".len())][..]
        );
        Ok(())
    }

    #[tokio::test]
    async fn lift_redact_roundtrip() -> Result<()> {
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = find_chunk(&h, r#"foo"bar"#);
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let source_loc = h
            .lift(&chunk, TextLocation::inline(value_start..value_end))
            .expect("range is in bounds");
        h.write_at(Redactions::from([(
            source_loc,
            TextReplacement::substituted("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"msg":"foo\"XXX"}"#);
        Ok(())
    }
}
