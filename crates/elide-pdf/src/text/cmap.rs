//! Scrub deleted glyph codes from a font's `/ToUnicode` CMap.
//!
//! A `/ToUnicode` CMap maps character codes to Unicode, so text can be recovered
//! from it (copy, search, extraction) independently of the drawn glyphs. When
//! [`redact_text`](crate::document::Pdf::redact_text) deletes a glyph, its code must also
//! be removed from this table, or the redacted text survives as a code->Unicode
//! entry. The CMap is edited textually: a `bfchar` entry for a deleted code is
//! dropped, and a `bfrange` covering one fails closed (its interior can't be
//! partially removed without re-deriving the range).

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, ErrorKind, Result};
use lopdf::{Document, Object, ObjectId};

/// Remove `deleted` codes from each named `/ToUnicode` CMap stream.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a deleted
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
            .map_err(|e| Error::new(ErrorKind::Redaction, format!("read /ToUnicode CMap: {e}")))?;
        let edited = strip_codes(&text, codes)?;

        let Ok(Object::Stream(stream)) = doc.get_object_mut(cmap_id) else {
            continue;
        };
        stream.set_plain_content(edited);
    }
    Ok(())
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
                return Err(Error::new(
                    ErrorKind::Redaction,
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
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
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
        assert_eq!(err.kind(), crate::ErrorKind::Redaction);
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
