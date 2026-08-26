//! JSON handler: a flat ordered sequence of source slots.
//!
//! The loader lexes the source once into [`Slot`]s: either
//! [`Slot::Passthrough`] (whitespace + structural punctuation, kept
//! verbatim) or [`Slot::Leaf`] (a key, string value, or scalar). Leaves
//! carry both the original source bytes (`serialized`) and the unescaped
//! UTF-8 value the recognizer sees (`value`). [`Handler::read_next`]
//! yields leaves in document order; `write_at` splices each redaction into
//! the leaf's `serialized` bytes at the mapped source span (keeping `value`
//! in sync); [`Handler::encode`] concatenates every slot.
//!
//! Because a redaction is spliced rather than re-rendered from the decoded
//! value, formatting (indentation, key order, whitespace) *and* every byte
//! outside a redacted span — including `\uXXXX` / `\"` escapes within a
//! redacted string — stay byte-identical to the source. The partial-leaf
//! offset translation is a single per-leaf walk of its escape table.

use std::ops::Range;

use elide_core::modality::text::{SourceRef, Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter, Hint};
use elide_core::operator::Redactions;
use elide_core::{Error, ErrorKind, Result};

use super::JsonLoader;
use super::json_escape::{decode_escape, json_escape};
use super::json_parser::parse_slots;
use crate::content::ContentData;
use crate::handler::redact;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the JSON codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.json");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), JsonLoader)
        .with_extensions(["json"])
        .with_content_types(["application/json"])
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
    /// not these bytes — the two differ wherever an escape collapses.
    pub serialized: String,
    /// Out-of-band located context (currently the enclosing object key)
    /// surfaced to recognizers as hints; empty for keys and for value
    /// leaves outside any object (e.g. a top-level scalar). Each hint
    /// carries the key's source span so a boost can point back at the key.
    pub hints: Vec<Hint<Text>>,
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
    /// editing `serialized` **in place** so every byte outside the edited span
    /// — quote, indentation, and any `\uXXXX` / `\"` escapes — survives
    /// verbatim, rather than re-rendering the whole value (which would flatten
    /// escapes to their literal chars). `value` is kept in sync so later reads
    /// and edits of the same leaf see the new content.
    fn splice(&mut self, value_range: Range<usize>, replacement: &str) -> Result<()> {
        if !self.is_quoted() {
            // A scalar has no escapes and no quotes; value and serialized are
            // the same bytes, so a direct splice of both keeps them in step.
            redact::replace_range(&mut self.value, replacement, value_range.clone())?;
            redact::replace_range(&mut self.serialized, replacement, value_range)?;
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
        let source_start = value_to_source_offset(self, 0, value_range.start)
            .ok_or_else(|| Error::new(ErrorKind::MalformedInput, "value offset out of range"))?;
        let source_end = value_to_source_offset(self, 0, value_range.end)
            .ok_or_else(|| Error::new(ErrorKind::MalformedInput, "value offset out of range"))?;
        redact::replace_range(
            &mut self.serialized,
            &json_escape(replacement),
            source_start..source_end,
        )?;
        redact::replace_range(&mut self.value, replacement, value_range)?;
        Ok(())
    }
}

/// Whether `s` is a bare JSON literal — a number, or one of `true` / `false`
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
/// `-?(0 | [1-9][0-9]*) (\.[0-9]+)? ([eE][+-]?[0-9]+)?` — an optional minus, an
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

/// Handler for loaded JSON content.
#[derive(Debug)]
pub(crate) struct JsonHandler {
    slots: Vec<Slot>,
    cursor: usize,
}

#[async_trait::async_trait]
impl Handler<Text> for JsonHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

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

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        while self.cursor < self.slots.len() {
            // The chunk's location addresses the decoded stream — its span
            // matches `data` (the decoded value), so a recognizer's offsets into
            // `data` map straight through. `lift` carries the raw span in
            // `.source` for a source-relative consumer.
            let start = self.decoded_offset_of(self.cursor);
            let slot = &self.slots[self.cursor];
            self.cursor += 1;
            if let Slot::Leaf(leaf) = slot {
                return Ok(Some(Chunk {
                    location: TextLocation::new(start, start + leaf.value.len()),
                    data: TextData::new(leaf.value.clone()),
                    hints: leaf.hints.clone(),
                }));
            }
        }
        Ok(None)
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // `local` is a byte range into this leaf's *decoded* value. The lifted
        // location's `start..end` addresses the decoded stream (so it stays in
        // step with the recognizer's own coordinates); `.source` carries the raw
        // span, since a JSON value differs from its source wherever an escape
        // (`\"`, `\uXXXX`) collapses.
        let (idx, leaf) = self.find_leaf(&chunk.location)?;
        let decoded_start = self.decoded_offset_of(idx) + local.range.start;
        let decoded_end = self.decoded_offset_of(idx) + local.range.end;

        // Raw span: map the value-local endpoints through the leaf's escape
        // table, offset by the leaf's serialized start in the document.
        let source_slot_start = self.source_offset_of(idx);
        let source_start = value_to_source_offset(leaf, source_slot_start, local.range.start)?;
        let source_end = value_to_source_offset(leaf, source_slot_start, local.range.end)?;

        Some(
            TextLocation::new(decoded_start, decoded_end)
                .with_page(chunk.location.page)
                .with_source([SourceRef::new(source_start..source_end)]),
        )
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for JsonHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        Ok(self
            .find_leaf(location)
            .map(|(_, leaf)| TextData::new(leaf.value.clone())))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for JsonHandler {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        // Resolve every redaction against the **pre-mutation** slot
        // offsets first. Mutating a leaf shifts every later slot's
        // source-byte offset, so resolving inline would mismatch later
        // locations against the live (already-shifted) slot table. The
        // plan stores per-leaf value-byte ranges, which stay valid
        // regardless of how other slots change length.
        let mut plan: Vec<(usize, usize, usize, String)> = Vec::new();
        let mut slot_offset = 0usize;
        let mut slot_iter = self.slots.iter().enumerate().peekable();
        let mut items: Vec<_> = redactions.into_iter().collect();
        items.sort_by_key(|(loc, _)| loc.range.start);
        for (loc, replacement) in items {
            // `loc` addresses the decoded stream (values interleaved with
            // passthrough). Advance the slot cursor to the slot containing it.
            // Decoded-stream offsets are monotonic, so a single forward sweep
            // resolves every redaction in O(slots + redactions).
            while let Some(&(idx, slot)) = slot_iter.peek() {
                let slot_end = slot_offset + slot.decoded_len();
                if loc.range.start < slot_end {
                    if let Slot::Leaf(_) = slot {
                        // Within a leaf the decoded stream *is* the leaf's value,
                        // so the decoded-local offsets are value offsets directly
                        // — no escape-walk on the read side (`Leaf::splice` maps
                        // the value range through the escapes on write).
                        let value_start = loc.range.start - slot_offset;
                        let value_end = loc.range.end - slot_offset;
                        let value = replacement.value().unwrap_or_default().to_owned();
                        plan.push((idx, value_start, value_end, value));
                    }
                    break;
                }
                slot_offset = slot_end;
                slot_iter.next();
            }
        }
        // Apply per-leaf edits right-to-left within each leaf so earlier
        // edits in the same leaf don't shift later ones.
        plan.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        for (idx, value_start, value_end, value) in plan {
            let Slot::Leaf(leaf) = &mut self.slots[idx] else {
                continue;
            };
            leaf.splice(value_start..value_end, &value)?;
        }
        self.cursor = 0;
        Ok(())
    }
}

impl JsonHandler {
    /// Build a handler directly from JSON source bytes. Used by the
    /// loader; preserves the source formatting verbatim.
    pub(super) fn from_source_string(source: String) -> Self {
        let slots = parse_slots(&source).unwrap_or_else(|_| vec![Slot::Passthrough(source)]);
        Self { slots, cursor: 0 }
    }

    /// The decoded-stream byte offset where the slot at `idx` starts.
    ///
    /// The decoded stream is what [`TextLocation`] offsets address: passthrough
    /// bytes verbatim (structural JSON carries no escapes) interleaved with each
    /// leaf's *decoded* [`value`](Leaf::value). It diverges from the serialized
    /// stream only across leaves whose value differs from its source (escapes).
    fn decoded_offset_of(&self, idx: usize) -> usize {
        self.slots[..idx].iter().map(Slot::decoded_len).sum()
    }

    /// The serialized byte offset where the slot at `idx` starts — what
    /// [`encode`](Handler::encode) emits and what a raw [`SourceRef`] addresses.
    fn source_offset_of(&self, idx: usize) -> usize {
        self.slots[..idx].iter().map(Slot::source_len).sum()
    }

    /// Locate the leaf slot whose *decoded* range contains `location`, returning
    /// its index and a borrow. `location` is a decoded-stream range.
    fn find_leaf(&self, location: &TextLocation) -> Option<(usize, &Leaf)> {
        let mut offset = 0usize;
        for (idx, slot) in self.slots.iter().enumerate() {
            let slot_end = offset + slot.decoded_len();
            if let Slot::Leaf(leaf) = slot
                && location.range.start >= offset
                && location.range.end <= slot_end
            {
                return Some((idx, leaf));
            }
            offset = slot_end;
        }
        None
    }
}

impl Slot {
    /// This slot's length in the decoded stream — the decoded value for a leaf,
    /// the verbatim bytes for passthrough.
    fn decoded_len(&self) -> usize {
        match self {
            Slot::Passthrough(t) => t.len(),
            Slot::Leaf(l) => l.value.len(),
        }
    }

    /// This slot's length in the serialized stream — the escaped source bytes
    /// for a leaf, the verbatim bytes for passthrough.
    fn source_len(&self) -> usize {
        match self {
            Slot::Passthrough(t) => t.len(),
            Slot::Leaf(l) => l.serialized.len(),
        }
    }
}

/// Map a value-byte offset inside `leaf.value` to the source byte offset
/// inside the current encoded output — the escape-aware translation from a
/// decoded position to its raw span, used by `lift` (for `.source`) and by
/// [`Leaf::splice`] (to place a redaction in the serialized bytes).
///
/// `slot_start` is the leaf's source byte offset. For scalars the mapping
/// is identity. For quoted leaves the value-byte cursor advances one for
/// each interior source byte (or two source bytes when the next source
/// byte is a `\` escape prefix).
///
/// Returns `None` if `value_offset` is past the end of the value.
fn value_to_source_offset(leaf: &Leaf, slot_start: usize, value_offset: usize) -> Option<usize> {
    if !leaf.is_quoted() {
        if value_offset > leaf.value.len() {
            return None;
        }
        return Some(slot_start + value_offset);
    }
    let escaped_start = slot_start + 1;
    let escaped_end = slot_start + leaf.serialized.len() - 1;
    let bytes = leaf.serialized.as_bytes();
    let mut src = escaped_start;
    let mut val = 0usize;
    while val < value_offset {
        if src >= escaped_end {
            return None;
        }
        let local = src - slot_start;
        let (source_len, value_len) = if bytes.get(local) == Some(&b'\\') {
            let (source_len, decoded) = decode_escape(&bytes[local..])?;
            (source_len, decoded.len_utf8())
        } else {
            (1, 1)
        };
        src += source_len;
        val += value_len;
    }
    Some(src)
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::TextReplacement;

    use super::*;

    fn handler(src: &str) -> JsonHandler {
        JsonHandler::from_source_string(src.to_string())
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
        let mut h = handler(r#"{"name":"Alice","age":30}"#);
        let mut chunks = Vec::new();
        while let Some(c) = h.read_next().await? {
            chunks.push(c.data.as_str().to_owned());
        }
        assert_eq!(chunks, vec!["name", "Alice", "age", "30"]);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_values_get_distinct_offsets() -> Result<()> {
        let mut h = handler(r#"{"a":"same","b":"same"}"#);
        let mut offsets = Vec::new();
        while let Some(c) = h.read_next().await? {
            if c.data.as_str() == "same" {
                offsets.push(c.location.range.start);
            }
        }
        assert_eq!(offsets.len(), 2);
        assert_ne!(offsets[0], offsets[1]);
        Ok(())
    }

    #[tokio::test]
    async fn read_returns_string() -> Result<()> {
        let mut h = handler(r#"{"name":"Alice"}"#);
        let mut found = false;
        while let Some(chunk) = h.read_next().await? {
            if h.read_at(&chunk.location)
                .await?
                .map(|d| d.as_str().to_owned())
                == Some("Alice".to_owned())
            {
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
        let chunk = loop {
            let c = h.read_next().await?.expect("expected chunk");
            if c.data.as_str() == "Alice" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location.clone(), TextReplacement::substituted("Bob"));
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"name":"Bob"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_partial_leaf_in_compact_source() -> Result<()> {
        let src = r#"{"email":"alice@example.com"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        // Decoded-stream offsets: the chunk's decoded start plus the value-local
        // range of "alice" (offset 0 in the value here).
        let value = chunk.data.as_str();
        let at = value.find("alice").unwrap();
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(
                chunk.location.range.start + at,
                chunk.location.range.start + at + "alice".len(),
            ),
            TextReplacement::substituted("[USER]"),
        );
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"email":"[USER]@example.com"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_partial_leaf_with_escapes() -> Result<()> {
        // decoded value: foo"bar; source: "foo\"bar" — redacting "bar" in the
        // decoded stream splices back over the source bytes past the `\"` escape.
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == r#"foo"bar"# {
                break c;
            }
        };
        let value = chunk.data.as_str();
        let at = value.find("bar").unwrap();
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(
                chunk.location.range.start + at,
                chunk.location.range.start + at + "bar".len(),
            ),
            TextReplacement::substituted("XXX"),
        );
        h.write_at(rs).await?;
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
        let mut h = handler(&src);
        let mut values = Vec::new();
        while let Some(c) = h.read_next().await? {
            values.push(c.data.as_str().to_owned());
        }
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
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == "caf\u{e9} bar" {
                break c;
            }
        };
        // Decoded-stream offsets: in the value `café bar`, the `é` is one char,
        // so `bar` sits at value byte 5 (`caf` = 3, `é` = 2 UTF-8 bytes). The
        // decoded location addresses it directly; write_at maps back across the
        // 6-byte `é` source escape when it splices.
        let value = chunk.data.as_str();
        let at = value.find("bar").unwrap();
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(
                chunk.location.range.start + at,
                chunk.location.range.start + at + "bar".len(),
            ),
            TextReplacement::substituted("XXX"),
        );
        h.write_at(rs).await?;
        // Only `bar` is replaced; the `é` escape before it is spliced through
        // verbatim (the redaction edits `serialized` in place, not a re-render of
        // the decoded value).
        assert_eq!(encoded(&h), format!("{{\"msg\":\"caf{} XXX\"}}", "\\u00e9"));
        Ok(())
    }

    #[tokio::test]
    async fn redact_key() -> Result<()> {
        let mut h = handler(r#"{"email":"a@b.c"}"#);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == "email" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(
            chunk.location.clone(),
            TextReplacement::substituted("contact"),
        );
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"contact":"a@b.c"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_scalar() -> Result<()> {
        let mut h = handler(r#"{"n":42}"#);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == "42" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location.clone(), TextReplacement::substituted("0"));
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"n":0}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_scalar_with_non_literal_replacement_quotes_it() -> Result<()> {
        // Masking a number with a non-literal string would make bare `XXX`
        // invalid JSON, so the leaf is promoted to a quoted string value.
        let mut h = handler(r#"{"n":42}"#);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == "42" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location.clone(), TextReplacement::substituted("XXX"));
        h.write_at(rs).await?;
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
        while let Some(c) = h.read_next().await? {
            let v = c.data.as_str();
            if v == "first" || v == "second" || v == "third" {
                locs.push(c.location);
            }
        }
        assert_eq!(locs.len(), 3, "expected three string values");
        let mut rs = Redactions::new();
        for loc in locs {
            rs.push(loc, TextReplacement::substituted("X"));
        }
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"a":"X","b":"X","c":"X"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn lift_simple_string() -> Result<()> {
        // No escapes: decoded and source coincide, so `range` and `.source`
        // agree — but `.source` is still populated with the raw span.
        let src = r#"{"email":"alice@example.com"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        let value_start = "alice@example.com".find("alice").unwrap();
        let value_end = value_start + "alice".len();
        let lifted = h
            .lift(&chunk, TextLocation::new(value_start, value_end))
            .expect("range is in bounds");
        // Primary range: decoded-stream offset (chunk start + value-local).
        assert_eq!(lifted.range.start, chunk.location.range.start + value_start);
        assert_eq!(lifted.range.end, chunk.location.range.start + value_end);
        // Source: the raw byte span of "alice" in the document.
        let raw = src.find("alice").unwrap();
        assert_eq!(
            lifted.source,
            vec![SourceRef::new(raw..raw + "alice".len())]
        );
        Ok(())
    }

    #[tokio::test]
    async fn lift_walks_escapes() -> Result<()> {
        // decoded value: foo"bar; source: "foo\"bar". The `\"` escape makes the
        // decoded stream one byte shorter than the source before "bar", so the
        // decoded `range` and the raw `.source` diverge — this is the case the
        // whole fix exists for.
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == r#"foo"bar"# {
                break c;
            }
        };
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let lifted = h
            .lift(&chunk, TextLocation::new(value_start, value_end))
            .expect("range is in bounds");
        // Decoded range: `bar` at value offset 4 (`foo"` = 4 decoded bytes).
        assert_eq!(lifted.range.start, chunk.location.range.start + value_start);
        assert_eq!(lifted.range.end, chunk.location.range.start + value_end);
        // Raw source span: `bar` sits after the 2-byte `\"` escape.
        let raw = src.find("bar").unwrap();
        assert_eq!(lifted.source, vec![SourceRef::new(raw..raw + "bar".len())]);
        Ok(())
    }

    #[tokio::test]
    async fn lift_redact_roundtrip() -> Result<()> {
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.read_next().await?.expect("chunk");
            if c.data.as_str() == r#"foo"bar"# {
                break c;
            }
        };
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let source_loc = h
            .lift(&chunk, TextLocation::new(value_start, value_end))
            .expect("range is in bounds");
        let mut rs = Redactions::new();
        rs.push(source_loc, TextReplacement::substituted("XXX"));
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"msg":"foo\"XXX"}"#);
        Ok(())
    }
}
