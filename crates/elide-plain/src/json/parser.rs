//! JSON lexer: a single forward pass over the source into its [`JsonItem`]s.
//!
//! [`parse_items`] drives an [`ItemParser`] that validates the whole document
//! and emits every key, string value, and scalar as an item: its decoded value
//! and the [`JsonSpan`] that maps it back onto the source bytes. Whitespace and
//! structural punctuation are not items; they stay in the source. Object keys
//! are surfaced as context hints on the values they label.

use elide_codec::extract::{OffsetMap, OffsetRun, Passage};
use elide_codec::string::ContextWords;
use elide_core::modality::ResolvedHint;
use elide_core::modality::text::{SourceRef, Text, TextData, TextLocation};
use elide_core::{Error, Result};

use super::escape::decode_escape;
use super::span::{JsonItem, JsonSpan};

/// Lex JSON source into its items, in document order.
///
/// # Errors
///
/// [`MalformedInput`] if the source is not well-formed JSON.
///
/// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
pub(super) fn parse_items(src: &str) -> Result<Vec<JsonItem>> {
    let mut p = ItemParser::new(src);
    p.parse_value(None)?;
    p.consume_whitespace();
    if p.pos != src.len() {
        return Err(Error::malformed(format!(
            "trailing bytes after JSON value at offset {}",
            p.pos
        )));
    }
    Ok(p.items)
}

/// Maximum object/array nesting depth accepted before the input is rejected.
/// `parse_object`/`parse_array` recurse once per level and the source is
/// caller-controlled, so an adversarial `[[[[…` would otherwise overflow the
/// stack, an abort a `Result` cannot catch. 128 is far beyond any real
/// document's structural depth.
const MAX_DEPTH: usize = 128;

struct ItemParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    /// Current object/array nesting depth, bounded by [`MAX_DEPTH`].
    depth: usize,
    items: Vec<JsonItem>,
}

impl<'a> ItemParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            depth: 0,
            items: Vec::new(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn consume_whitespace(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn consume_punct(&mut self, c: u8) -> Result<()> {
        if self.peek() != Some(c) {
            return Err(Error::malformed(format!(
                "expected {:?} at offset {}",
                char::from(c),
                self.pos
            )));
        }
        self.pos += 1;
        Ok(())
    }

    /// After a member of an object or array: consume the `,` before the next
    /// one, returning `true`, or the `close` that ends it, returning `false`.
    fn separator(&mut self, close: u8) -> Result<bool> {
        self.consume_whitespace();
        match self.peek() {
            Some(b',') => self.consume_punct(b',').map(|()| true),
            Some(b) if b == close => self.consume_punct(close).map(|()| false),
            _ => Err(Error::malformed(format!(
                "expected ',' or {:?} at offset {}",
                char::from(close),
                self.pos
            ))),
        }
    }

    /// Run `parse` (an object/array body) one level deeper, restoring the
    /// depth afterwards. The caller has already checked `depth < MAX_DEPTH`.
    fn nested(&mut self, parse: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn parse_value(&mut self, key_context: Option<&ResolvedHint<Text>>) -> Result<()> {
        self.consume_whitespace();
        match self.peek() {
            Some(b'{' | b'[') if self.depth >= MAX_DEPTH => Err(Error::malformed(format!(
                "JSON nested deeper than {MAX_DEPTH} at offset {}",
                self.pos
            ))),
            Some(b'{') => self.nested(Self::parse_object),
            Some(b'[') => self.nested(|p| p.parse_array(key_context)),
            Some(b'"' | b't' | b'f' | b'n' | b'-' | b'0'..=b'9') => {
                let mut item = if self.peek() == Some(b'"') {
                    self.parse_string()?
                } else {
                    self.parse_scalar()?
                };
                item.hints.extend(key_context.cloned());
                self.items.push(item);
                Ok(())
            }
            Some(b) => Err(Error::malformed(format!(
                "unexpected byte {b:#x} at offset {}",
                self.pos
            ))),
            None => Err(Error::malformed("unexpected end of input")),
        }
    }

    fn parse_object(&mut self) -> Result<()> {
        self.consume_punct(b'{')?;
        self.consume_whitespace();
        if self.peek() == Some(b'}') {
            return self.consume_punct(b'}');
        }
        loop {
            self.consume_whitespace();
            // The key's source span is the quoted form `"…"` between here
            // and where `parse_string` leaves the cursor; that span is the
            // located hint we hand every value under this key. The hint text
            // is the key split into words (`paymentCard` → `payment card`), so
            // a context keyword like `card` matches on a word boundary, the
            // JSON counterpart of an XML element name or a CSV header vouching
            // for its value.
            let key_start = self.pos;
            let key = self.parse_string()?;
            // The key span is a raw *source* range, so the hint is a
            // source-only location, resolved against the source, not the value
            // it labels.
            let key_hint = ResolvedHint::new(
                TextLocation::marked_source(SourceRef::new(key_start..self.pos)),
                TextData::new(key.value.context_words()),
            );
            self.items.push(key);
            self.consume_whitespace();
            self.consume_punct(b':')?;
            self.parse_value(Some(&key_hint))?;
            if !self.separator(b'}')? {
                return Ok(());
            }
        }
    }

    fn parse_array(&mut self, key_context: Option<&ResolvedHint<Text>>) -> Result<()> {
        self.consume_punct(b'[')?;
        self.consume_whitespace();
        if self.peek() == Some(b']') {
            return self.consume_punct(b']');
        }
        loop {
            // Array elements inherit the containing object key as their
            // hint: `{"cards": ["4111…", "5555…"]}` should treat both
            // PANs as living under `cards`.
            self.parse_value(key_context)?;
            if !self.separator(b']')? {
                return Ok(());
            }
        }
    }

    /// A quoted string at the cursor: its decoded value, each escape mapped
    /// atomically onto its source spelling and each literal character onto
    /// itself.
    fn parse_string(&mut self) -> Result<JsonItem> {
        self.consume_punct(b'"')?;
        let start = self.pos;
        let mut value = String::new();
        let mut runs = Vec::new();
        // Where the current stretch of literal characters starts.
        let mut literal = Literal {
            source: start,
            value: 0,
        };
        loop {
            let at = self.pos;
            match self.peek() {
                Some(b'"') => {
                    literal.end(&mut runs, at, value.len());
                    self.pos += 1;
                    let span = JsonSpan {
                        range: start..at,
                        offsets: OffsetMap::new(runs),
                        quoted: true,
                        read: value.clone(),
                    };
                    return Ok(item(span, value));
                }
                Some(b'\\') => {
                    literal.end(&mut runs, at, value.len());
                    let escape = decode_escape(&self.bytes[at..]).ok_or_else(|| {
                        Error::malformed(format!("invalid escape at offset {at}"))
                    })?;
                    let decoded = value.len();
                    value.push(escape.character);
                    self.pos += escape.len;
                    runs.push(OffsetRun::entity(decoded..value.len(), at..self.pos));
                    literal = Literal {
                        source: self.pos,
                        value: value.len(),
                    };
                }
                Some(b) if b < 0x20 => {
                    return Err(Error::malformed(format!(
                        "unescaped control character in a JSON string at offset {at}"
                    )));
                }
                Some(_) => {
                    // `pos` is on a character boundary: every step advances
                    // by a whole character or escape.
                    let ch = self.src[at..]
                        .chars()
                        .next()
                        .ok_or_else(|| Error::malformed("unterminated string"))?;
                    value.push(ch);
                    self.pos += ch.len_utf8();
                }
                None => return Err(Error::malformed("unterminated string")),
            }
        }
    }

    fn parse_scalar(&mut self) -> Result<JsonItem> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'+' | b'.' | b'_')) {
                break;
            }
            self.pos += 1;
        }
        let literal = &self.src[start..self.pos];
        // The scalar byte class (`alnum | - | + | . | _`) is permissive and
        // would accept non-JSON runs like `tru`, `1.2.3`, `+5`, or `_`. Reject
        // anything that is not a well-formed JSON literal so the lexer honors
        // its contract of erroring on malformed input.
        if !is_json_literal(literal) {
            return Err(Error::malformed(format!(
                "invalid JSON literal at offset {start}"
            )));
        }
        let span = JsonSpan {
            range: start..self.pos,
            offsets: OffsetMap::identity(start, literal.len()),
            quoted: false,
            read: literal.to_owned(),
        };
        Ok(item(span, literal.to_owned()))
    }
}

/// Where a stretch of a string's literal characters starts: they read the
/// same in the source and the value.
#[derive(Debug, Clone, Copy)]
struct Literal {
    /// Its source offset.
    source: usize,
    /// Its value offset.
    value: usize,
}

impl Literal {
    /// End the stretch at source offset `source` and value offset `value`,
    /// pushing it onto `runs` as an identity run; nothing when it is empty.
    fn end(self, runs: &mut Vec<OffsetRun>, source: usize, value: usize) {
        if self.source < source {
            runs.push(OffsetRun::identity(self.value..value, self.source..source));
        }
    }
}

/// A JSON item over `span` reading `value`: its own passage, unhinted.
fn item(span: JsonSpan, value: String) -> JsonItem {
    JsonItem {
        address: span,
        value,
        hints: Vec::new(),
        passage: Passage::Start,
        flag: None,
    }
}

/// Whether `s` is a bare JSON literal, a number, or one of `true` / `false`
/// / `null`. The lexer uses it to reject a malformed scalar, and the write-back
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
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    // Skip the digits from `i`, returning how many there were.
    let digits = |i: &mut usize| {
        let start = *i;
        while matches!(bytes.get(*i), Some(b'0'..=b'9')) {
            *i += 1;
        }
        *i - start
    };
    // Integer part: a lone `0`, or a non-zero digit followed by more digits.
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            digits(&mut i);
        }
        _ => return false,
    }
    // Optional fraction: a dot followed by at least one digit.
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    }
    // Optional exponent: e/E, optional sign, at least one digit.
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    // Anything left over (`007`'s second `0`, `1.2.3`'s `.3`) is no number.
    i == bytes.len()
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    #[test]
    fn rejects_input_nested_past_the_depth_limit() {
        // Deeply nested arrays must error, not overflow the stack.
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 5), "]".repeat(MAX_DEPTH + 5));
        let err = parse_items(&deep).expect_err("over-deep nesting should be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[test]
    fn accepts_input_at_the_depth_limit() {
        // Exactly MAX_DEPTH levels is fine and round-trips.
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse_items(&ok).is_ok(), "MAX_DEPTH nesting should parse");
    }

    #[test]
    fn rejects_a_raw_control_character_in_a_string() {
        let err = parse_items("[\"a\u{1}b\"]").expect_err("raw U+0001 is not JSON");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert!(
            parse_items(r#"["a\u0001b"]"#).is_ok(),
            "the escaped form is"
        );
    }

    /// The error names the offset, never the document's bytes.
    #[test]
    fn a_bad_literal_is_not_quoted_in_the_error() {
        let err = parse_items("[123456789x]").expect_err("not a number");
        assert!(!err.to_string().contains("123456789"), "{err}");
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
            let err = parse_items(bad).expect_err("{bad} must be rejected");
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "for {bad}");
        }
        // Valid literals still parse: keywords, ints, floats, exponents, signs.
        for ok in ["[true]", "[false]", "[null]", "[42]", "[-3.14]", "[1e10]"] {
            assert!(parse_items(ok).is_ok(), "{ok} should parse");
        }
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

    /// A string's value reads decoded, and each escape maps onto its whole
    /// spelling.
    #[test]
    fn a_string_maps_each_escape_onto_its_spelling() -> Result<()> {
        let src = format!(r#"["a\"b{}c"]"#, "\\u00e9");
        let items = parse_items(&src)?;
        let [item] = items.as_slice() else {
            panic!("one item: {items:?}");
        };
        assert_eq!(item.value, "a\"b\u{e9}c");
        let offsets = &item.address.offsets;
        // `"` at value 1 is `\"` at source 3..5; `é` at 3..5 is `\u00e9` at
        // 6..12.
        assert_eq!(offsets.raw_ranges(1..2), vec![3..5]);
        assert_eq!(offsets.raw_ranges(3..5), vec![6..12]);
        assert_eq!(offsets.raw_ranges(0..6), vec![2..13]);
        Ok(())
    }
}
