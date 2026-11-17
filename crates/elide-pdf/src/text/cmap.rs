//! Edits to a font's `/ToUnicode` CMap.
//!
//! A `/ToUnicode` CMap maps character codes to Unicode, so text can be recovered
//! from it (copy, search, extraction) independently of the drawn glyphs. When
//! [`delete`](crate::text::redact::delete) deletes a glyph, its code must also
//! be removed from this table, or the redacted text survives as a code->Unicode
//! entry. The CMap is edited textually: a `bfchar` entry for a deleted code is
//! dropped, and a `bfrange` covering one fails closed (its interior can't be
//! partially removed without re-deriving the range). The rebuild also writes
//! each destination as [`readable`] text, so the text a viewer copies is the
//! text detection read.

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, Result};
use lopdf::{Document, Object, ObjectId};

use super::glyphs::readable;

/// The most codes a `bfrange` is rewritten across; a CMap's ranges span at
/// most 256 (their bounds differ only in the last byte).
const MAX_RANGE: u64 = 1 << 16;

/// Remove `deleted` codes from each named `/ToUnicode` CMap stream.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a deleted
/// code falls inside a multi-code `bfrange` (which cannot be scrubbed without
/// re-deriving the range) or if a CMap stream cannot be read, so the redaction
/// fails closed rather than leaving a recoverable code->Unicode entry.
pub(crate) fn scrub(
    doc: &mut Document,
    deleted: &BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
    surviving: &BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
) -> Result<()> {
    for (&cmap_id, deleted_here) in deleted {
        // Spare any code still drawn by surviving text (shared fonts recur the
        // same code in text that stays): only scrub codes used nowhere else.
        let empty = BTreeSet::new();
        let kept = surviving.get(&cmap_id).unwrap_or(&empty);
        let codes: BTreeSet<Vec<u8>> = deleted_here.difference(kept).cloned().collect();
        if codes.is_empty() {
            continue;
        }
        let codes = &codes;
        let Ok(Object::Stream(stream)) = doc.get_object(cmap_id) else {
            // No readable stream: nothing to scrub here (the font may map codes
            // some other way); the glyph bytes are already gone from the page.
            continue;
        };
        let text = stream
            .decompressed_content()
            .map_err(|e| Error::redaction(format!("read /ToUnicode CMap: {e}")))?;
        let edited = strip_codes(&text, codes)?;

        let Ok(Object::Stream(stream)) = doc.get_object_mut(cmap_id) else {
            continue;
        };
        stream.set_plain_content(edited);
    }
    Ok(())
}

/// `cmap` with each Unicode destination of its `bfchar` and `bfrange` sections
/// written as [`readable`] text, or `None` when none changes.
///
/// A range whose destinations change is written as the array of them (a
/// range's destinations step its first one's last UTF-16 unit, as
/// [`lopdf`] reads them). A destination that is not UTF-16 stays as written.
pub(crate) fn readable_cmap(cmap: &[u8]) -> Option<Vec<u8>> {
    // Latin-1 maps each byte to one char and back, so tokens outside the
    // destinations keep their exact bytes.
    let text: String = cmap.iter().map(|&b| char::from(b)).collect();
    let tokens = tokenize(&text);
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut section = "";
    let mut i = 0;
    while i < tokens.len() {
        let word = tokens[i].word();
        if matches!(
            word,
            "beginbfchar" | "beginbfrange" | "endbfchar" | "endbfrange"
        ) {
            section = if word.starts_with("begin") { word } else { "" };
            out.push_str(tokens[i].text);
            i += 1;
            continue;
        }
        let entry = match section {
            "beginbfchar" => readable_bfchar(&tokens[i..]),
            "beginbfrange" => readable_bfrange(&tokens[i..]),
            _ => None,
        };
        match entry {
            Some((written, used)) => {
                changed = true;
                out.push_str(&written);
                i += used;
            }
            None => {
                out.push_str(tokens[i].text);
                i += 1;
            }
        }
    }
    changed.then(|| out.chars().map(|c| c as u8).collect())
}

/// The `<src> <dst>` entry at the start of `tokens` with `dst` rewritten, and
/// the tokens it spans; `None` when it is not one or does not change.
fn readable_bfchar(tokens: &[Token<'_>]) -> Option<(String, usize)> {
    let [src, dst, ..] = tokens else {
        return None;
    };
    src.hex_bytes()?;
    let units = utf16_units(&dst.hex_bytes()?)?;
    let written = readable_units(&units)?;
    Some((
        format!("{}{}{}", src.text, leading(dst.text), hex(&written)),
        2,
    ))
}

/// The `<lo> <hi> dst` entry at the start of `tokens` with its destinations
/// rewritten, and the tokens it spans; `None` when it is not one or none of
/// its destinations changes.
fn readable_bfrange(tokens: &[Token<'_>]) -> Option<(String, usize)> {
    let [lo, hi, dst, ..] = tokens else {
        return None;
    };
    let low = code(&lo.hex_bytes()?)?;
    let high = code(&hi.hex_bytes()?)?;
    let count = high.checked_sub(low)? + 1;
    let prefix = format!("{}{}{}", lo.text, hi.text, leading(dst.text));
    if let Some(bytes) = dst.hex_bytes() {
        if count > MAX_RANGE {
            return None;
        }
        let first = utf16_units(&bytes)?;
        let mut changed = false;
        let mut entries = Vec::new();
        for step in 0..count {
            let mut units = first.clone();
            let last = units.last_mut()?;
            *last = last.wrapping_add(step as u16);
            match readable_units(&units) {
                Some(written) => {
                    changed = true;
                    entries.push(hex(&written));
                }
                None => entries.push(hex(&units)),
            }
        }
        return changed.then(|| (format!("{prefix}[{}]", entries.join(" ")), 3));
    }
    let inner = dst.word().strip_prefix('[')?.strip_suffix(']')?;
    let mut changed = false;
    let mut written = String::from("[");
    for token in tokenize(inner) {
        let rewritten = token
            .hex_bytes()
            .and_then(|bytes| utf16_units(&bytes))
            .and_then(|units| readable_units(&units));
        match rewritten {
            Some(units) => {
                changed = true;
                written.push_str(leading(token.text));
                written.push_str(&hex(&units));
            }
            None => written.push_str(token.text),
        }
    }
    written.push(']');
    changed.then(|| (format!("{prefix}{written}"), 3))
}

/// The UTF-16 `units` written as [`readable`] text, or `None` when that is
/// what they already say or they are not UTF-16.
fn readable_units(units: &[u16]) -> Option<Vec<u16>> {
    let text = String::from_utf16(units).ok()?;
    let written = readable(&text);
    (written != text).then(|| written.encode_utf16().collect())
}

/// `bytes` as big-endian UTF-16 units, or `None` for an odd length.
fn utf16_units(bytes: &[u8]) -> Option<Vec<u16>> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    rest.is_empty()
        .then(|| pairs.iter().map(|&pair| u16::from_be_bytes(pair)).collect())
}

/// A source code's bytes as a number, or `None` past four bytes.
fn code(bytes: &[u8]) -> Option<u64> {
    (bytes.len() <= 4).then(|| bytes.iter().fold(0, |code, &b| code << 8 | u64::from(b)))
}

/// UTF-16 `units` as a `<hex>` string.
fn hex(units: &[u16]) -> String {
    let digits: String = units.iter().map(|unit| format!("{unit:04X}")).collect();
    format!("<{digits}>")
}

/// The whitespace a token's `text` starts with.
fn leading(text: &str) -> &str {
    &text[..text.len() - text.trim_start().len()]
}

/// Remove the `bfchar` entries whose source code is in `codes`, and fail closed
/// if any code falls inside a multi-code `bfrange`.
///
/// CMap syntax is PostScript-like, not line-oriented: `beginbfchar`/`endbfchar`
/// and several `<src> <dst>` pairs may share a line, so the stream is tokenized
/// rather than split on newlines. Inside a `bfchar` section a pair whose source
/// is a deleted code is dropped; every other token is re-emitted verbatim. A
/// deleted code inside a genuine (multi-code) `bfrange` cannot be excised
/// textually, so the whole rewrite fails closed rather than leaving the mapping.
fn strip_codes(cmap: &[u8], codes: &BTreeSet<Vec<u8>>) -> Result<Vec<u8>> {
    let text = String::from_utf8_lossy(cmap);
    let tokens = tokenize(&text);
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < tokens.len() {
        let tok = &tokens[i];
        match tok.word() {
            "beginbfchar" => {
                out.push_str(tok.text);
                i += 1;
                i = copy_bfchar_section(&tokens, i, codes, &mut out)?;
            }
            "beginbfrange" => {
                out.push_str(tok.text);
                i += 1;
                i = copy_bfrange_section(&tokens, i, codes, &mut out)?;
            }
            _ => {
                out.push_str(tok.text);
                i += 1;
            }
        }
    }
    Ok(out.into_bytes())
}

/// Copy a `bfchar` section, dropping any `<src> <dst>` pair whose source is a
/// deleted code. Returns the index just past `endbfchar`.
fn copy_bfchar_section(
    tokens: &[Token<'_>],
    mut i: usize,
    codes: &BTreeSet<Vec<u8>>,
    out: &mut String,
) -> Result<usize> {
    while i < tokens.len() && tokens[i].word() != "endbfchar" {
        // A `bfchar` entry is `<src> <dst>`. Read the source hex; if the next
        // token isn't a second hex the section is malformed, re-emit and move on.
        let src = tokens[i].hex_bytes();
        if let (Some(src), Some(dst)) = (src, tokens.get(i + 1))
            && dst.hex_bytes().is_some()
        {
            if codes.contains(&src) {
                // Drop both tokens of this pair.
                i += 2;
                continue;
            }
            out.push_str(tokens[i].text);
            out.push_str(tokens[i + 1].text);
            i += 2;
            continue;
        }
        out.push_str(tokens[i].text);
        i += 1;
    }
    if i < tokens.len() {
        out.push_str(tokens[i].text); // endbfchar
        i += 1;
    }
    Ok(i)
}

/// Copy a `bfrange` section verbatim, failing closed if a deleted code falls in
/// a genuine range (a degenerate `lo == hi` range that is deleted is dropped).
/// Returns the index just past `endbfrange`.
fn copy_bfrange_section(
    tokens: &[Token<'_>],
    mut i: usize,
    codes: &BTreeSet<Vec<u8>>,
    out: &mut String,
) -> Result<usize> {
    while i < tokens.len() && tokens[i].word() != "endbfrange" {
        // A `bfrange` entry is `<lo> <hi> <dst-or-array>`. Inspect lo/hi.
        let lo = tokens[i].hex_bytes();
        let hi = tokens.get(i + 1).and_then(Token::hex_bytes);
        if let (Some(lo), Some(hi)) = (lo, hi) {
            let hit = codes.iter().any(|c| *c >= lo && *c <= hi);
            if hit {
                if lo == hi && codes.contains(&lo) {
                    // Degenerate single-code range: drop lo, hi, and the target
                    // token that follows.
                    i += if tokens.get(i + 2).is_some() { 3 } else { 2 };
                    continue;
                }
                return Err(Error::redaction(
                    "a deleted glyph's code falls inside a /ToUnicode bfrange; \
                     the code->Unicode mapping cannot be scrubbed safely",
                ));
            }
        }
        out.push_str(tokens[i].text);
        i += 1;
    }
    if i < tokens.len() {
        out.push_str(tokens[i].text); // endbfrange
        i += 1;
    }
    Ok(i)
}

/// One lexical token of a CMap stream, carrying its exact source slice (so
/// re-emitting a token preserves the original bytes and whitespace).
struct Token<'a> {
    /// The token's source text, including any leading whitespace, so the tokens
    /// concatenated reproduce the input minus the dropped ones.
    text: &'a str,
    /// The token kind, so consumers can recognize hex strings.
    kind: TokenKind,
}

/// The kinds of CMap token this scrubber distinguishes.
enum TokenKind {
    /// A `<hex>` string.
    Hex,
    /// Anything else (keyword, name, integer, array, delimiter, whitespace run).
    Other,
}

impl Token<'_> {
    /// The token's text with its leading whitespace trimmed, for matching
    /// keywords (`beginbfchar`, `endbfchar`, ...).
    fn word(&self) -> &str {
        self.text.trim()
    }

    /// The decoded bytes of a `<hex>` token, or `None` for any other kind.
    fn hex_bytes(&self) -> Option<Vec<u8>> {
        if !matches!(self.kind, TokenKind::Hex) {
            return None;
        }
        let inner = self.text.trim().strip_prefix('<')?.strip_suffix('>')?;
        hex_to_bytes(inner)
    }
}

/// Tokenize a CMap stream into tokens whose concatenated `text` reproduces the
/// input exactly. Leading whitespace is attached to the token that follows it,
/// so re-emitting the kept tokens preserves layout. A `<hex>` string is one
/// token; an array `[...]` is kept whole; everything else is a whitespace-
/// delimited word.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        // Attach the leading whitespace run to the next token.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            // Trailing whitespace: emit it as an `Other` token so it survives.
            tokens.push(Token {
                text: &text[start..i],
                kind: TokenKind::Other,
            });
            break;
        }
        let kind = match bytes[i] {
            b'<' => {
                // A hex string runs to the closing `>`.
                while i < bytes.len() && bytes[i] != b'>' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // include `>`
                }
                TokenKind::Hex
            }
            b'[' => {
                // An array runs to the closing `]`.
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // include `]`
                }
                TokenKind::Other
            }
            _ => {
                // A word runs to the next whitespace or delimiter.
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'<'
                    && bytes[i] != b'['
                {
                    i += 1;
                }
                TokenKind::Other
            }
        };
        tokens.push(Token {
            text: &text[start..i],
            kind,
        });
    }
    tokens
}

/// Decode an even-length hex string to bytes; `None` if malformed.
fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.trim();
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for &[hi, lo] in bytes.as_chunks::<2>().0 {
        let hi = (hi as char).to_digit(16)?;
        let lo = (lo as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    fn codes(list: &[&[u8]]) -> BTreeSet<Vec<u8>> {
        list.iter().map(|c| c.to_vec()).collect()
    }

    #[test]
    fn drops_a_bfchar_entry_for_a_deleted_code() {
        let cmap = b"begincmap\n2 beginbfchar\n<0041> <0041>\n<0042> <0042>\nendbfchar\nendcmap\n";
        let out = strip_codes(cmap, &codes(&[&[0x00, 0x41]])).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("<0041> <0041>"), "deleted entry survived");
        assert!(s.contains("<0042> <0042>"), "untouched entry lost");
    }

    #[test]
    fn keeps_unrelated_entries() {
        let cmap = b"1 beginbfchar\n<0042> <0042>\nendbfchar\n";
        let out = strip_codes(cmap, &codes(&[&[0x00, 0x41]])).unwrap();
        assert_eq!(out, cmap.to_vec());
    }

    #[test]
    fn drops_a_degenerate_single_code_bfrange() {
        let cmap = b"1 beginbfrange\n<0041> <0041> <0041>\nendbfrange\n";
        let out = strip_codes(cmap, &codes(&[&[0x00, 0x41]])).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("<0041> <0041> <0041>"));
    }

    #[test]
    fn fails_closed_on_a_code_inside_a_real_bfrange() {
        let cmap = b"1 beginbfrange\n<0041> <005A> <0041>\nendbfrange\n";
        let err = strip_codes(cmap, &codes(&[&[0x00, 0x42]])).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    #[test]
    fn drops_the_right_pair_when_several_share_a_line() {
        // Three pairs on ONE line; the deleted code is the middle one. A
        // line-oriented scrubber would keep the whole line (leak); the token
        // scrubber drops only the middle pair.
        let cmap = b"beginbfchar <0041> <0041> <0042> <0042> <0043> <0043> endbfchar\n";
        let out = strip_codes(cmap, &codes(&[&[0x00, 0x42]])).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("<0042> <0042>"), "deleted middle pair leaked");
        assert!(s.contains("<0041> <0041>"), "first pair lost");
        assert!(s.contains("<0043> <0043>"), "last pair lost");
    }

    #[test]
    fn drops_a_pair_from_a_fully_inline_section() {
        // `beginbfchar`, the pair, and `endbfchar` all on one line.
        let cmap = b"begincmap beginbfchar <0041> <0041> endbfchar endcmap\n";
        let out = strip_codes(cmap, &codes(&[&[0x00, 0x41]])).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("<0041>"), "inline deleted pair leaked");
        assert!(s.contains("endcmap"), "surrounding structure lost");
    }

    #[test]
    fn writes_bfchar_destinations_as_readable_text() {
        // Chrome maps 山 to the Kangxi radical ⼭ (U+2F2D); Quartz maps fi to ﬁ.
        let cmap = b"2 beginbfchar\n<0681> <2F2D>\n<0003> <FB01>\nendbfchar\n";
        let out = String::from_utf8(readable_cmap(cmap).unwrap()).unwrap();
        assert_eq!(
            out,
            "2 beginbfchar\n<0681> <5C71>\n<0003> <00660069>\nendbfchar\n"
        );
    }

    #[test]
    fn leaves_a_cmap_without_forms_alone() {
        let cmap = b"1 beginbfchar\n<0041> <0041>\nendbfchar\n1 beginbfrange\n<01> <03> <0061>\nendbfrange\n";
        assert_eq!(readable_cmap(cmap), None);
    }

    #[test]
    fn writes_a_changed_range_as_its_destinations() {
        // <2F2B>..<2F2D> are the radicals for 尸, 屮 and 山.
        let cmap = b"1 beginbfrange\n<10> <12> <2F2B>\nendbfrange\n";
        let out = String::from_utf8(readable_cmap(cmap).unwrap()).unwrap();
        assert_eq!(
            out,
            "1 beginbfrange\n<10> <12> [<5C38> <5C6E> <5C71>]\nendbfrange\n"
        );
    }

    #[test]
    fn writes_a_range_array_entry_by_entry() {
        let cmap = b"beginbfrange <10> <11> [<0041> <2F2D>] endbfrange";
        let out = String::from_utf8(readable_cmap(cmap).unwrap()).unwrap();
        assert_eq!(out, "beginbfrange <10> <11> [<0041> <5C71>] endbfrange");
    }

    #[test]
    fn keeps_bytes_outside_the_destinations() {
        let cmap = b"/CMapName /\xE9 def\nbeginbfchar <01> <2F2D> endbfchar";
        let out = readable_cmap(cmap).unwrap();
        assert_eq!(
            out,
            b"/CMapName /\xE9 def\nbeginbfchar <01> <5C71> endbfchar"
        );
    }

    #[test]
    fn tokenize_round_trips_the_input() {
        // Concatenating every token reproduces the input exactly (so kept
        // tokens preserve the original layout).
        let cmap = "begincmap\n2 beginbfchar\n<0041> <0041>\nendbfchar\nendcmap\n";
        let joined: String = tokenize(cmap).iter().map(|t| t.text).collect();
        assert_eq!(joined, cmap);
    }

    #[test]
    fn hex_decodes() {
        assert_eq!(hex_to_bytes("0041"), Some(vec![0x00, 0x41]));
        assert_eq!(hex_to_bytes("41"), Some(vec![0x41]));
        assert_eq!(hex_to_bytes("XYZ"), None);
        assert_eq!(hex_to_bytes("041"), None);
    }
}
