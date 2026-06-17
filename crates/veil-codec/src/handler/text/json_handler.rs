//! JSON handler: a flat ordered sequence of source slots.
//!
//! The loader lexes the source once into [`Slot`]s — either
//! [`Slot::Passthrough`] (whitespace + structural punctuation, kept
//! verbatim) or [`Slot::Leaf`] (a key, string value, or scalar). Leaves
//! carry both the original source bytes (`serialized`) and the unescaped
//! UTF-8 value the recognizer sees (`value`). [`Handler::next_chunk`]
//! yields leaves in document order; `write_at` mutates the leaf's value
//! and re-renders its serialized form; [`Handler::encode`] concatenates
//! every slot.
//!
//! This keeps formatting (indentation, key order, whitespace)
//! byte-identical to the source for any slot the caller didn't touch, and
//! reduces the partial-leaf offset translation to a single per-leaf walk
//! of its escape table.

use std::ops::Range;

use veil_core::modality::text::{Text, TextData, TextLocation};
use veil_core::modality::{DataReader, DataWriter};
use veil_core::redaction::Redactions;
use veil_core::{Error, ErrorKind};

use crate::content::ContentData;
use crate::handler::redact;
use crate::{Chunk, Format, FormatId, Handler};

/// Stable [`FormatId`] for the JSON codec.
pub const FORMAT_ID: FormatId = FormatId::from_static("veil.text.json");

/// [`Format`] descriptor registered into [`crate::CodecRegistry`].
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), super::JsonLoader)
        .with_extensions(["json"])
        .with_content_types(["application/json"])
}

/// One element of the parsed source.
#[derive(Debug, Clone)]
pub(super) enum Slot {
    /// Whitespace or structural punctuation (`{ } [ ] : ,` and
    /// surrounding whitespace). Held verbatim and emitted back unchanged.
    Passthrough(String),
    /// A key, string value, or scalar (number/bool/null) — every position
    /// a recognizer is allowed to address.
    Leaf(Leaf),
}

/// An addressable position in the document.
#[derive(Debug, Clone)]
pub(super) struct Leaf {
    pub kind: LeafKind,
    /// Current unescaped UTF-8 value — what the recognizer sees in
    /// [`Chunk::data`] and what redactions edit.
    pub value: String,
    /// Current source bytes — what `encode` emits and what
    /// [`TextLocation`] offsets address. For [`LeafKind::Key`] and
    /// [`LeafKind::StringValue`] this is the quoted form `"…"` with `\\` /
    /// `\"` escapes; for [`LeafKind::Scalar`] it is the bare literal.
    pub serialized: String,
    /// Out-of-band context strings (currently the enclosing object key)
    /// surfaced to recognizers as hints; empty for keys and for value
    /// leaves outside any object (e.g. a top-level scalar).
    pub hints: Vec<String>,
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

    fn render(&mut self) {
        self.serialized = match self.kind {
            LeafKind::Key | LeafKind::StringValue => format!("\"{}\"", json_escape(&self.value)),
            LeafKind::Scalar => self.value.clone(),
        };
    }
}

/// Handler for loaded JSON content.
#[derive(Debug)]
pub struct JsonHandler {
    slots: Vec<Slot>,
    cursor: usize,
}

impl Handler<Text> for JsonHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData, Error> {
        let mut out = String::new();
        for slot in &self.slots {
            match slot {
                Slot::Passthrough(text) => out.push_str(text),
                Slot::Leaf(leaf) => out.push_str(&leaf.serialized),
            }
        }
        Ok(ContentData::from_text(out))
    }

    async fn next_chunk(&mut self) -> Result<Option<Chunk<Text>>, Error> {
        while self.cursor < self.slots.len() {
            let start = self.offset_of(self.cursor);
            let slot = &self.slots[self.cursor];
            self.cursor += 1;
            if let Slot::Leaf(leaf) = slot {
                return Ok(Some(Chunk {
                    location: TextLocation {
                        start,
                        end: start + leaf.serialized.len(),
                        ..Default::default()
                    },
                    data: TextData::new(leaf.value.clone()),
                    hints: leaf.hints.clone(),
                }));
            }
        }
        Ok(None)
    }

    fn lift_chunk(&self, chunk: &Chunk<Text>, value_range: Range<usize>) -> Option<TextLocation> {
        let (idx, leaf) = self.find_leaf(&chunk.location)?;
        let slot_start = self.offset_of(idx);
        let source_start = value_to_source_offset(leaf, slot_start, value_range.start)?;
        let source_end = value_to_source_offset(leaf, slot_start, value_range.end)?;
        Some(TextLocation {
            start: source_start,
            end: source_end,
            page: chunk.location.page,
        })
    }
}

impl DataReader<Text> for JsonHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>, Error> {
        Ok(self
            .find_leaf(location)
            .map(|(_, leaf)| TextData::new(leaf.value.clone())))
    }
}

impl DataWriter<Text> for JsonHandler {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<(), Error> {
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
        items.sort_by_key(|(loc, _)| loc.start);
        for (loc, replacement) in items {
            // Advance the slot cursor to the slot containing `loc`. Slot
            // offsets are monotonic, so a single forward sweep resolves
            // every redaction in O(slots + redactions).
            while let Some(&(idx, slot)) = slot_iter.peek() {
                let len = match slot {
                    Slot::Passthrough(t) => t.len(),
                    Slot::Leaf(l) => l.serialized.len(),
                };
                let slot_end = slot_offset + len;
                if loc.start < slot_end {
                    if let Slot::Leaf(leaf) = slot
                        && let Some((value_start, value_end)) =
                            translate_to_value(leaf, slot_offset, loc.start, loc.end)
                    {
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
            redact::replace_range(&mut leaf.value, &value, value_start..value_end)?;
            leaf.render();
        }
        self.cursor = 0;
        Ok(())
    }
}

impl JsonHandler {
    /// Build a handler from a synthetic [`serde_json::Value`]. Synthetic
    /// documents always re-emit compact JSON — loaded documents preserve
    /// their source formatting via the slot model.
    pub fn from_value(value: serde_json::Value) -> Self {
        let serialized = serde_json::to_string(&value).unwrap_or_default();
        Self::from_source_string(serialized)
    }

    /// Build a handler directly from JSON source bytes. Used by the
    /// loader; preserves the source formatting verbatim.
    pub(super) fn from_source_string(source: String) -> Self {
        let slots = parse_slots(&source).unwrap_or_else(|_| vec![Slot::Passthrough(source)]);
        Self { slots, cursor: 0 }
    }

    /// Rewind the streaming cursor to the start of the document.
    pub fn rewind(&mut self) {
        self.cursor = 0;
    }

    /// Byte offset where the slot at `idx` starts in the current encoded
    /// output.
    fn offset_of(&self, idx: usize) -> usize {
        self.slots[..idx]
            .iter()
            .map(|s| match s {
                Slot::Passthrough(t) => t.len(),
                Slot::Leaf(l) => l.serialized.len(),
            })
            .sum()
    }

    /// Locate the leaf slot whose source range contains `location`.
    /// Returns its index and a borrow.
    fn find_leaf(&self, location: &TextLocation) -> Option<(usize, &Leaf)> {
        let mut offset = 0usize;
        for (idx, slot) in self.slots.iter().enumerate() {
            let len = match slot {
                Slot::Passthrough(t) => t.len(),
                Slot::Leaf(l) => l.serialized.len(),
            };
            let slot_end = offset + len;
            if let Slot::Leaf(leaf) = slot
                && location.start >= offset
                && location.end <= slot_end
            {
                return Some((idx, leaf));
            }
            offset = slot_end;
        }
        None
    }
}

/// Escape a string for JSON matching (backslash and quote only — other
/// control characters in keys/values are unsupported in this codec's
/// redaction path and round-trip as-is).
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Translate a `(source_start, source_end)` range expressed in the
/// current encoded output into the corresponding `(value_start,
/// value_end)` range inside `leaf.value`.
///
/// `slot_start` is the leaf's byte offset in the current output. For
/// scalars the mapping is identity. For quoted leaves the boundary
/// positions (opening/closing quote) map to the full value range;
/// interior positions are translated by walking the `\\`/`\"` escape
/// pairs.
///
/// Returns `None` when the requested boundary lands on a quote byte
/// (outside the whole-leaf case) or in the middle of an escape pair.
fn translate_to_value(
    leaf: &Leaf,
    slot_start: usize,
    source_start: usize,
    source_end: usize,
) -> Option<(usize, usize)> {
    let slot_end = slot_start + leaf.serialized.len();
    if !leaf.is_quoted() {
        return Some((source_start - slot_start, source_end - slot_start));
    }
    if source_start == slot_start && source_end == slot_end {
        return Some((0, leaf.value.len()));
    }
    let escaped_start = slot_start + 1;
    let escaped_end = slot_end - 1;
    if source_start < escaped_start || source_end > escaped_end || source_start > source_end {
        return None;
    }
    let bytes = leaf.serialized.as_bytes();
    let mut src = escaped_start;
    let mut val = 0usize;
    let mut start = None;
    let mut end = None;
    while src <= escaped_end {
        if src == source_start {
            start = Some(val);
        }
        if src == source_end {
            end = Some(val);
            break;
        }
        // Index into leaf.serialized: `src - slot_start`.
        let local = src - slot_start;
        let is_escape = bytes.get(local) == Some(&b'\\');
        if is_escape && src + 1 > escaped_end {
            return None;
        }
        src += if is_escape { 2 } else { 1 };
        val += 1;
    }
    Some((start?, end?))
}

/// Inverse of [`translate_to_value`]: map a value-byte offset inside
/// `leaf.value` to the source byte offset inside the current encoded
/// output.
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
        let is_escape = bytes.get(local) == Some(&b'\\');
        src += if is_escape { 2 } else { 1 };
        val += 1;
    }
    Some(src)
}

/// Lex JSON source into a flat ordered slot list.
///
/// Whitespace and structural punctuation collapse into
/// [`Slot::Passthrough`]; keys, string values and scalars become
/// [`Slot::Leaf`]. Returns an error if the source is not well-formed
/// JSON.
pub(super) fn parse_slots(src: &str) -> Result<Vec<Slot>, Error> {
    let mut p = SlotParser::new(src);
    p.parse_value(None)?;
    p.flush_passthrough();
    p.consume_whitespace();
    p.flush_passthrough();
    if p.pos != src.len() {
        return Err(Error::new(
            ErrorKind::Validation,
            format!("trailing bytes after JSON value at offset {}", p.pos),
        ));
    }
    Ok(p.slots)
}

struct SlotParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    /// Pending whitespace + structural bytes we haven't flushed into a
    /// [`Slot::Passthrough`] yet.
    pending: String,
    slots: Vec<Slot>,
}

impl<'a> SlotParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            pending: String::new(),
            slots: Vec::new(),
        }
    }

    fn flush_passthrough(&mut self) {
        if !self.pending.is_empty() {
            self.slots
                .push(Slot::Passthrough(std::mem::take(&mut self.pending)));
        }
    }

    fn push_leaf(&mut self, leaf: Leaf) {
        self.flush_passthrough();
        self.slots.push(Slot::Leaf(leaf));
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn consume_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pending.push(b as char);
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn consume_punct(&mut self, c: u8) -> Result<(), Error> {
        if self.peek() != Some(c) {
            return Err(Error::new(
                ErrorKind::Validation,
                format!("expected {:?} at offset {}", c as char, self.pos),
            ));
        }
        self.pending.push(c as char);
        self.pos += 1;
        Ok(())
    }

    fn parse_value(&mut self, key_context: Option<&str>) -> Result<(), Error> {
        self.consume_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(key_context),
            Some(b'"') => {
                let mut leaf = self.parse_string_leaf(LeafKind::StringValue)?;
                if let Some(k) = key_context {
                    leaf.hints.push(k.to_owned());
                }
                self.push_leaf(leaf);
                Ok(())
            }
            Some(b't') | Some(b'f') | Some(b'n') | Some(b'-') | Some(b'0'..=b'9') => {
                let mut leaf = self.parse_scalar()?;
                if let Some(k) = key_context {
                    leaf.hints.push(k.to_owned());
                }
                self.push_leaf(leaf);
                Ok(())
            }
            Some(b) => Err(Error::new(
                ErrorKind::Validation,
                format!("unexpected byte {b:#x} at offset {}", self.pos),
            )),
            None => Err(Error::new(
                ErrorKind::Validation,
                "unexpected end of input".to_string(),
            )),
        }
    }

    fn parse_object(&mut self) -> Result<(), Error> {
        self.consume_punct(b'{')?;
        self.consume_whitespace();
        if self.peek() == Some(b'}') {
            self.consume_punct(b'}')?;
            return Ok(());
        }
        loop {
            self.consume_whitespace();
            let key = self.parse_string_leaf(LeafKind::Key)?;
            let key_value = key.value.clone();
            self.push_leaf(key);
            self.consume_whitespace();
            self.consume_punct(b':')?;
            self.parse_value(Some(&key_value))?;
            self.consume_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.consume_punct(b',')?;
                }
                Some(b'}') => {
                    self.consume_punct(b'}')?;
                    return Ok(());
                }
                _ => {
                    return Err(Error::new(
                        ErrorKind::Validation,
                        format!("expected ',' or '}}' at offset {}", self.pos),
                    ));
                }
            }
        }
    }

    fn parse_array(&mut self, key_context: Option<&str>) -> Result<(), Error> {
        self.consume_punct(b'[')?;
        self.consume_whitespace();
        if self.peek() == Some(b']') {
            self.consume_punct(b']')?;
            return Ok(());
        }
        loop {
            // Array elements inherit the containing object key as their
            // hint — `{"cards": ["4111…", "5555…"]}` should treat both
            // PANs as living under `cards`.
            self.parse_value(key_context)?;
            self.consume_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.consume_punct(b',')?;
                }
                Some(b']') => {
                    self.consume_punct(b']')?;
                    return Ok(());
                }
                _ => {
                    return Err(Error::new(
                        ErrorKind::Validation,
                        format!("expected ',' or ']' at offset {}", self.pos),
                    ));
                }
            }
        }
    }

    fn parse_string_leaf(&mut self, kind: LeafKind) -> Result<Leaf, Error> {
        if self.peek() != Some(b'"') {
            return Err(Error::new(
                ErrorKind::Validation,
                format!("expected '\"' at offset {}", self.pos),
            ));
        }
        let start = self.pos;
        self.pos += 1;
        let mut value = String::new();
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    let serialized = self.src[start..self.pos].to_string();
                    return Ok(Leaf {
                        kind,
                        value,
                        serialized,
                        hints: Vec::new(),
                    });
                }
                Some(b'\\') => {
                    let next = self.bytes.get(self.pos + 1).copied();
                    match next {
                        Some(b'"') => value.push('"'),
                        Some(b'\\') => value.push('\\'),
                        Some(b'/') => value.push('/'),
                        Some(b'n') => value.push('\n'),
                        Some(b'r') => value.push('\r'),
                        Some(b't') => value.push('\t'),
                        Some(b'b') => value.push('\u{0008}'),
                        Some(b'f') => value.push('\u{000c}'),
                        Some(b'u') => {
                            return Err(Error::new(
                                ErrorKind::Validation,
                                format!(
                                    "JSON \\u escapes are not supported in redaction codec \
                                     (offset {})",
                                    self.pos
                                ),
                            ));
                        }
                        Some(other) => {
                            return Err(Error::new(
                                ErrorKind::Validation,
                                format!(
                                    "invalid escape \\{} at offset {}",
                                    other as char, self.pos
                                ),
                            ));
                        }
                        None => {
                            return Err(Error::new(
                                ErrorKind::Validation,
                                "unterminated escape at end of input".to_string(),
                            ));
                        }
                    }
                    self.pos += 2;
                }
                Some(_) => {
                    let ch_start = self.pos;
                    // Advance one UTF-8 codepoint without reading the
                    // escape table.
                    let rest = &self.src[ch_start..];
                    let ch = rest.chars().next().ok_or_else(|| {
                        Error::new(ErrorKind::Validation, "unterminated string".to_string())
                    })?;
                    value.push(ch);
                    self.pos += ch.len_utf8();
                }
                None => {
                    return Err(Error::new(
                        ErrorKind::Validation,
                        "unterminated string".to_string(),
                    ));
                }
            }
        }
    }

    fn parse_scalar(&mut self) -> Result<Leaf, Error> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            let is_scalar_byte =
                b.is_ascii_alphanumeric() || matches!(b, b'-' | b'+' | b'.' | b'_');
            if is_scalar_byte {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(Error::new(
                ErrorKind::Validation,
                format!("expected scalar at offset {start}"),
            ));
        }
        let literal = self.src[start..self.pos].to_string();
        Ok(Leaf {
            kind: LeafKind::Scalar,
            value: literal.clone(),
            serialized: literal,
            hints: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use veil_core::modality::text::TextReplacement;

    use super::*;

    fn handler(src: &str) -> JsonHandler {
        JsonHandler::from_source_string(src.to_string())
    }

    fn encoded(h: &JsonHandler) -> String {
        h.encode().unwrap().decode().unwrap()
    }

    #[tokio::test]
    async fn stream_yields_keys_and_values_in_order() -> Result<(), Error> {
        let mut h = handler(r#"{"name":"Alice","age":30}"#);
        let mut chunks = Vec::new();
        while let Some(c) = h.next_chunk().await? {
            chunks.push(c.data.as_str().to_owned());
        }
        assert_eq!(chunks, vec!["name", "Alice", "age", "30"]);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_values_get_distinct_offsets() -> Result<(), Error> {
        let mut h = handler(r#"{"a":"same","b":"same"}"#);
        let mut offsets = Vec::new();
        while let Some(c) = h.next_chunk().await? {
            if c.data.as_str() == "same" {
                offsets.push(c.location.start);
            }
        }
        assert_eq!(offsets.len(), 2);
        assert_ne!(offsets[0], offsets[1]);
        Ok(())
    }

    #[tokio::test]
    async fn read_returns_string() -> Result<(), Error> {
        let mut h = handler(r#"{"name":"Alice"}"#);
        let mut found = false;
        while let Some(chunk) = h.next_chunk().await? {
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
    fn encode_preserves_source_compact() -> Result<(), Error> {
        let src = r#"{"a":1}"#;
        assert_eq!(encoded(&handler(src)), src);
        Ok(())
    }

    #[test]
    fn encode_preserves_source_pretty() -> Result<(), Error> {
        let src = "{\n  \"a\": 1\n}\n";
        assert_eq!(encoded(&handler(src)), src);
        Ok(())
    }

    #[tokio::test]
    async fn redact_whole_string_value() -> Result<(), Error> {
        let mut h = handler(r#"{"name":"Alice"}"#);
        let chunk = loop {
            let c = h.next_chunk().await?.expect("expected chunk");
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
    async fn redact_partial_leaf_in_compact_source() -> Result<(), Error> {
        let src = r#"{"email":"alice@example.com"}"#;
        let mut h = handler(src);
        let _ = loop {
            let c = h.next_chunk().await?.expect("chunk");
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        let local_start = src.find("alice").unwrap();
        let local_end = local_start + "alice".len();
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(local_start, local_end),
            TextReplacement::substituted("[USER]"),
        );
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"email":"[USER]@example.com"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_partial_leaf_with_escapes() -> Result<(), Error> {
        // unescaped value: foo"bar — source: "foo\"bar"
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let _ = loop {
            let c = h.next_chunk().await?.expect("chunk");
            if c.data.as_str() == r#"foo"bar"# {
                break c;
            }
        };
        let local_start = src.find("bar").unwrap();
        let local_end = local_start + "bar".len();
        let mut rs = Redactions::new();
        rs.push(
            TextLocation::new(local_start, local_end),
            TextReplacement::substituted("XXX"),
        );
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"msg":"foo\"XXX"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_key() -> Result<(), Error> {
        let mut h = handler(r#"{"email":"a@b.c"}"#);
        let chunk = loop {
            let c = h.next_chunk().await?.expect("chunk");
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
    async fn redact_scalar() -> Result<(), Error> {
        let mut h = handler(r#"{"n":42}"#);
        let chunk = loop {
            let c = h.next_chunk().await?.expect("chunk");
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

    /// Multiple redactions in a single batch, each targeting a different
    /// leaf, with length deltas that shift later slot offsets. Regression
    /// test for the "only the first redaction lands" bug.
    #[tokio::test]
    async fn redact_multiple_leaves_with_shifting_offsets() -> Result<(), Error> {
        let src = r#"{"a":"first","b":"second","c":"third"}"#;
        let mut h = handler(src);
        let mut locs = Vec::new();
        while let Some(c) = h.next_chunk().await? {
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
    async fn lift_chunk_simple_string() -> Result<(), Error> {
        let src = r#"{"email":"alice@example.com"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.next_chunk().await?.expect("chunk");
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        let value_start = "alice@example.com".find("alice").unwrap();
        let value_end = value_start + "alice".len();
        let source_loc = h
            .lift_chunk(&chunk, value_start..value_end)
            .expect("range is in bounds");
        let expected_start = src.find("alice").unwrap();
        assert_eq!(source_loc.start, expected_start);
        assert_eq!(source_loc.end, expected_start + "alice".len());
        Ok(())
    }

    #[tokio::test]
    async fn lift_chunk_walks_escapes() -> Result<(), Error> {
        // unescaped value: foo"bar — source: "foo\"bar"
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.next_chunk().await?.expect("chunk");
            if c.data.as_str() == r#"foo"bar"# {
                break c;
            }
        };
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let source_loc = h
            .lift_chunk(&chunk, value_start..value_end)
            .expect("range is in bounds");
        let expected_start = src.find("bar").unwrap();
        assert_eq!(source_loc.start, expected_start);
        assert_eq!(source_loc.end, expected_start + "bar".len());
        Ok(())
    }

    #[tokio::test]
    async fn lift_chunk_redact_roundtrip() -> Result<(), Error> {
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = loop {
            let c = h.next_chunk().await?.expect("chunk");
            if c.data.as_str() == r#"foo"bar"# {
                break c;
            }
        };
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let source_loc = h
            .lift_chunk(&chunk, value_start..value_end)
            .expect("range is in bounds");
        let mut rs = Redactions::new();
        rs.push(source_loc, TextReplacement::substituted("XXX"));
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"msg":"foo\"XXX"}"#);
        Ok(())
    }
}
