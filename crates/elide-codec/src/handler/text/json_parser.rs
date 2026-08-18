//! JSON lexer: a single forward pass over the source into a flat, ordered
//! [`Slot`] list.
//!
//! [`parse_slots`] drives a [`SlotParser`] that collapses whitespace and
//! structural punctuation into [`Slot::Passthrough`] and emits every key,
//! string value, and scalar as a [`Slot::Leaf`] carrying both its raw
//! `serialized` bytes and its decoded `value`. Object keys are surfaced as
//! context hints on the values they label.

use std::mem;

use elide_core::modality::Hint;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::{Error, ErrorKind, Result};

use super::json_escape::decode_escape;
use super::json_handler::{Leaf, LeafKind, Slot, is_json_literal};
use crate::handler::context::context_words;

/// Lex JSON source into a flat ordered slot list.
///
/// Whitespace and structural punctuation collapse into
/// [`Slot::Passthrough`]; keys, string values and scalars become
/// [`Slot::Leaf`]. Returns an error if the source is not well-formed
/// JSON.
pub(super) fn parse_slots(src: &str) -> Result<Vec<Slot>> {
    let mut p = SlotParser::new(src);
    p.parse_value(None)?;
    p.flush_passthrough();
    p.consume_whitespace();
    p.flush_passthrough();
    if p.pos != src.len() {
        return Err(Error::new(
            ErrorKind::MalformedInput,
            format!("trailing bytes after JSON value at offset {}", p.pos),
        ));
    }
    Ok(p.slots)
}

/// Maximum object/array nesting depth accepted before the input is rejected.
/// `parse_object`/`parse_array` recurse once per level and the source is
/// caller-controlled, so an adversarial `[[[[…` would otherwise overflow the
/// stack — an abort a `Result` cannot catch. 128 is far beyond any real
/// document's structural depth.
const MAX_DEPTH: usize = 128;

struct SlotParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    /// Current object/array nesting depth, bounded by [`MAX_DEPTH`].
    depth: usize,
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
            depth: 0,
            pending: String::new(),
            slots: Vec::new(),
        }
    }

    fn flush_passthrough(&mut self) {
        if !self.pending.is_empty() {
            self.slots
                .push(Slot::Passthrough(mem::take(&mut self.pending)));
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

    fn consume_punct(&mut self, c: u8) -> Result<()> {
        if self.peek() != Some(c) {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("expected {:?} at offset {}", c as char, self.pos),
            ));
        }
        self.pending.push(c as char);
        self.pos += 1;
        Ok(())
    }

    /// Run `parse` (an object/array body) one level deeper, restoring the
    /// depth afterwards. The caller has already checked `depth < MAX_DEPTH`.
    fn nested(&mut self, parse: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn parse_value(&mut self, key_context: Option<&Hint<Text>>) -> Result<()> {
        self.consume_whitespace();
        match self.peek() {
            Some(b'{') | Some(b'[') if self.depth >= MAX_DEPTH => Err(Error::new(
                ErrorKind::MalformedInput,
                format!("JSON nested deeper than {MAX_DEPTH} at offset {}", self.pos),
            )),
            Some(b'{') => self.nested(Self::parse_object),
            Some(b'[') => self.nested(|p| p.parse_array(key_context)),
            Some(b'"') => {
                let mut leaf = self.parse_string_leaf(LeafKind::StringValue)?;
                if let Some(k) = key_context {
                    leaf.hints.push(k.clone());
                }
                self.push_leaf(leaf);
                Ok(())
            }
            Some(b't') | Some(b'f') | Some(b'n') | Some(b'-') | Some(b'0'..=b'9') => {
                let mut leaf = self.parse_scalar()?;
                if let Some(k) = key_context {
                    leaf.hints.push(k.clone());
                }
                self.push_leaf(leaf);
                Ok(())
            }
            Some(b) => Err(Error::new(
                ErrorKind::MalformedInput,
                format!("unexpected byte {b:#x} at offset {}", self.pos),
            )),
            None => Err(Error::new(
                ErrorKind::MalformedInput,
                "unexpected end of input".to_string(),
            )),
        }
    }

    fn parse_object(&mut self) -> Result<()> {
        self.consume_punct(b'{')?;
        self.consume_whitespace();
        if self.peek() == Some(b'}') {
            self.consume_punct(b'}')?;
            return Ok(());
        }
        loop {
            self.consume_whitespace();
            // The key's source span is the quoted form `"…"` between here
            // and where `parse_string_leaf` leaves the cursor; that span is
            // the located hint we hand every value under this key. The hint
            // text is the key split into words (`paymentCard` → `payment
            // card`), so a context keyword like `card` matches on a word
            // boundary — the JSON counterpart of an XML element name or a CSV
            // header vouching for its value.
            let key_start = self.pos;
            let key = self.parse_string_leaf(LeafKind::Key)?;
            let key_hint = Hint::new(
                TextLocation::new(key_start, self.pos),
                TextData::new(context_words(&key.value)),
            );
            self.push_leaf(key);
            self.consume_whitespace();
            self.consume_punct(b':')?;
            self.parse_value(Some(&key_hint))?;
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
                        ErrorKind::MalformedInput,
                        format!("expected ',' or '}}' at offset {}", self.pos),
                    ));
                }
            }
        }
    }

    fn parse_array(&mut self, key_context: Option<&Hint<Text>>) -> Result<()> {
        self.consume_punct(b'[')?;
        self.consume_whitespace();
        if self.peek() == Some(b']') {
            self.consume_punct(b']')?;
            return Ok(());
        }
        loop {
            // Array elements inherit the containing object key as their
            // hint: `{"cards": ["4111…", "5555…"]}` should treat both
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
                        ErrorKind::MalformedInput,
                        format!("expected ',' or ']' at offset {}", self.pos),
                    ));
                }
            }
        }
    }

    fn parse_string_leaf(&mut self, kind: LeafKind) -> Result<Leaf> {
        if self.peek() != Some(b'"') {
            return Err(Error::new(
                ErrorKind::MalformedInput,
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
                    let (source_len, decoded) =
                        decode_escape(&self.bytes[self.pos..]).ok_or_else(|| {
                            Error::new(
                                ErrorKind::MalformedInput,
                                format!("invalid escape at offset {}", self.pos),
                            )
                        })?;
                    value.push(decoded);
                    self.pos += source_len;
                }
                Some(_) => {
                    let ch_start = self.pos;
                    // Advance one UTF-8 codepoint without reading the
                    // escape table.
                    let rest = &self.src[ch_start..];
                    let ch = rest.chars().next().ok_or_else(|| {
                        Error::new(ErrorKind::MalformedInput, "unterminated string".to_string())
                    })?;
                    value.push(ch);
                    self.pos += ch.len_utf8();
                }
                None => {
                    return Err(Error::new(
                        ErrorKind::MalformedInput,
                        "unterminated string".to_string(),
                    ));
                }
            }
        }
    }

    fn parse_scalar(&mut self) -> Result<Leaf> {
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
                ErrorKind::MalformedInput,
                format!("expected scalar at offset {start}"),
            ));
        }
        let literal = self.src[start..self.pos].to_string();
        // The scalar byte class (`alnum | - | + | . | _`) is permissive and
        // would accept non-JSON runs like `tru`, `1.2.3`, `+5`, or `_`. Reject
        // anything that is not a well-formed JSON literal so the lexer honors
        // its contract of erroring on malformed input.
        if !is_json_literal(&literal) {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("invalid JSON literal {literal:?} at offset {start}"),
            ));
        }
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
    use super::*;

    #[test]
    fn rejects_input_nested_past_the_depth_limit() {
        // Deeply nested arrays must error, not overflow the stack.
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 5), "]".repeat(MAX_DEPTH + 5));
        let err = parse_slots(&deep).expect_err("over-deep nesting should be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[test]
    fn accepts_input_at_the_depth_limit() {
        // Exactly MAX_DEPTH levels is fine and round-trips.
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse_slots(&ok).is_ok(), "MAX_DEPTH nesting should parse");
    }

    #[test]
    fn rejects_scalars_that_are_not_valid_json_literals() {
        // The permissive scalar byte class must not let malformed literals
        // through: partial keywords, malformed numbers, and bare `_` are not
        // JSON values.
        // Includes forms Rust's f64 parser accepts but JSON rejects: a
        // trailing dot, a leading zero, and the infinities.
        for bad in [
            "[tru]", "[1.2.3]", "[+5]", "[0x1f]", "[_]", "[nul]", "[5.]", "[007]", "[-inf]",
        ] {
            let err = parse_slots(bad).expect_err("{bad} must be rejected");
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "for {bad}");
        }
        // Valid literals still parse: keywords, ints, floats, exponents, signs.
        for ok in ["[true]", "[false]", "[null]", "[42]", "[-3.14]", "[1e10]"] {
            assert!(parse_slots(ok).is_ok(), "{ok} should parse");
        }
    }
}
