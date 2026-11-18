//! Edits to a font's `/ToUnicode` CMap.
//!
//! A `/ToUnicode` CMap maps character codes to Unicode, so text can be recovered
//! from it (copy, search, extraction) independently of the drawn glyphs. When
//! text redaction deletes a glyph, its code must also
//! be removed from this table, or the redacted text survives as a code->Unicode
//! entry. The CMap is edited textually: a `bfchar` entry for a deleted code is
//! dropped, and a `bfrange` covering one fails closed (its interior can't be
//! partially removed without re-deriving the range). The rebuild also writes
//! each destination as [`readable`] text, so the text a viewer copies is the
//! text detection read.

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, Result};
use lopdf::{Document, Object, ObjectId};

use crate::pdf::reader::font::readable;

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
///
/// Each rewrite's growth over what it replaces is taken from `budget`: a
/// range of a few bytes expands to all its destinations.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the
/// rewrites grow the CMap by more than `budget` bytes.
pub(crate) fn readable_cmap(cmap: &[u8], budget: &mut usize) -> Result<Option<Vec<u8>>> {
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
            "beginbfrange" => readable_bfrange(&tokens[i..], *budget)?,
            _ => None,
        };
        let Some(Entry { used, written }) = entry else {
            out.push_str(tokens[i].text);
            i += 1;
            continue;
        };
        let spanned = &tokens[i..i + used];
        i += used;
        let Some(written) = written else {
            spanned.iter().for_each(|token| out.push_str(token.text));
            continue;
        };
        let replaced: usize = spanned.iter().map(|token| token.text.chars().count()).sum();
        let growth = written.chars().count().saturating_sub(replaced);
        *budget = budget.checked_sub(growth).ok_or_else(|| {
            Error::resource_limit("the /ToUnicode CMaps rewrite past their size limit")
        })?;
        changed = true;
        out.push_str(&written);
    }
    Ok(changed.then(|| out.chars().map(|c| c as u8).collect()))
}

/// One `bfchar` or `bfrange` entry: the tokens it spans, and what it is
/// rewritten as when any of its destinations changes.
struct Entry {
    used: usize,
    written: Option<String>,
}

/// The `<src> <dst>` entry at the start of `tokens`, its `dst` rewritten when
/// that changes it; `None` when it is not one.
fn readable_bfchar(tokens: &[Token<'_>]) -> Option<Entry> {
    let [src, dst, ..] = tokens else {
        return None;
    };
    src.hex_bytes()?;
    if !dst.is_destination() {
        return None;
    }
    let written = dst
        .hex_bytes()
        .and_then(|bytes| utf16_units(&bytes))
        .and_then(|units| readable_units(&units))
        .map(|written| format!("{}{}{}", src.text, leading(dst.text), hex(&written)));
    Some(Entry { used: 2, written })
}

/// The `<lo> <hi> dst` entry at the start of `tokens`, rewritten when any of
/// its destinations changes; `None` when it is not one.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the
/// rewrite grows the entry by more than `budget` bytes.
fn readable_bfrange(tokens: &[Token<'_>], budget: usize) -> Result<Option<Entry>> {
    let [lo, hi, dst, ..] = tokens else {
        return Ok(None);
    };
    if lo.hex_bytes().is_none() || hi.hex_bytes().is_none() || !dst.is_destination() {
        return Ok(None);
    }
    let replaced: usize = [lo, hi, dst].iter().map(|t| t.text.chars().count()).sum();
    Ok(Some(Entry {
        used: 3,
        written: readable_range(lo, hi, dst, replaced.saturating_add(budget))?,
    }))
}

/// The range `lo`..=`hi` mapped from `dst` with its destinations rewritten,
/// or `None` when none of them changes.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) once the
/// destinations it expands to pass `limit` bytes, before more are held.
fn readable_range(
    lo: &Token<'_>,
    hi: &Token<'_>,
    dst: &Token<'_>,
    limit: usize,
) -> Result<Option<String>> {
    let (Some(low), Some(high)) = (
        lo.hex_bytes().and_then(|b| code(&b)),
        hi.hex_bytes().and_then(|b| code(&b)),
    ) else {
        return Ok(None);
    };
    let Some(count) = high.checked_sub(low).map(|span| span + 1) else {
        return Ok(None);
    };
    let prefix = format!("{}{}{}", lo.text, hi.text, leading(dst.text));
    if let Some(bytes) = dst.hex_bytes() {
        let Some(first) = utf16_units(&bytes).filter(|units| !units.is_empty()) else {
            return Ok(None);
        };
        if count > MAX_RANGE {
            return Ok(None);
        }
        let units = |step: u64| {
            let mut units = first.clone();
            if let Some(last) = units.last_mut() {
                *last = last.wrapping_add(step as u16);
            }
            units
        };
        if !(0..count).any(|step| readable_units(&units(step)).is_some()) {
            return Ok(None);
        }
        let mut size = prefix.chars().count() + 2;
        let mut entries = Vec::new();
        for step in 0..count {
            let units = units(step);
            let entry = hex(&readable_units(&units).unwrap_or(units));
            size += entry.len() + 1;
            if size > limit {
                return Err(Error::resource_limit(
                    "the /ToUnicode CMaps rewrite past their size limit",
                ));
            }
            entries.push(entry);
        }
        return Ok(Some(format!("{prefix}[{}]", entries.join(" "))));
    }
    let Some(inner) = dst
        .word()
        .strip_prefix('[')
        .and_then(|d| d.strip_suffix(']'))
    else {
        return Ok(None);
    };
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
    Ok(changed.then(|| format!("{prefix}{written}")))
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

/// The whitespace and comments a token's `text` starts with.
fn leading(text: &str) -> &str {
    &text[..trivia(text.as_bytes(), 0)]
}

/// The index past the whitespace and comments (`%` to the end of the line)
/// starting at `i` in `bytes`.
fn trivia(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
        } else if bytes[i] == b'%' {
            while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                i += 1;
            }
        } else {
            break;
        }
    }
    i
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
        let dst = tokens.get(i + 2).filter(|dst| dst.is_destination());
        if let (Some(lo), Some(hi)) = (lo, hi) {
            if codes.iter().any(|c| *c >= lo && *c <= hi) {
                if lo == hi && codes.contains(&lo) && dst.is_some() {
                    // Degenerate single-code range: drop lo, hi, and the target
                    // token that follows.
                    i += 3;
                    continue;
                }
                return Err(Error::redaction(
                    "a deleted glyph's code falls inside a /ToUnicode bfrange; \
                     the code->Unicode mapping cannot be scrubbed safely",
                ));
            }
            if dst.is_some() {
                tokens[i..i + 3]
                    .iter()
                    .for_each(|token| out.push_str(token.text));
                i += 3;
                continue;
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
    /// The token's source text, including any leading whitespace and
    /// comments, so the tokens concatenated reproduce the input minus the
    /// dropped ones.
    text: &'a str,
    /// The token kind, so consumers can recognize hex strings.
    kind: TokenKind,
}

/// The kinds of CMap token this scrubber distinguishes.
enum TokenKind {
    /// A `<hex>` string.
    Hex,
    /// Anything else (keyword, name, integer, string, array, or the
    /// whitespace and comments the input ends with).
    Other,
}

impl Token<'_> {
    /// The token's text past its leading whitespace and comments, for
    /// matching keywords (`beginbfchar`, `endbfchar`, ...).
    fn word(&self) -> &str {
        &self.text[leading(self.text).len()..]
    }

    /// Whether the token can be an entry's destination: a `<hex>` string, a
    /// name, or an array.
    fn is_destination(&self) -> bool {
        matches!(self.kind, TokenKind::Hex) || self.word().starts_with(['/', '['])
    }

    /// The decoded bytes of a `<hex>` token, or `None` for any other kind.
    fn hex_bytes(&self) -> Option<Vec<u8>> {
        if !matches!(self.kind, TokenKind::Hex) {
            return None;
        }
        let inner = self.word().strip_prefix('<')?.strip_suffix('>')?;
        hex_to_bytes(inner)
    }
}

/// Tokenize a CMap stream into tokens whose concatenated `text` reproduces the
/// input exactly. Leading whitespace and comments are attached to the token
/// that follows them, so re-emitting the kept tokens preserves layout and no
/// comment is read as an entry. A `<hex>` string is one token; a `(string)`
/// and an array `[...]` are kept whole; everything else is a word.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        // Attach the leading whitespace and comments to the next token.
        i = trivia(bytes, i);
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
            b'(' => {
                // A string runs to its balancing `)`, past escaped bytes.
                let mut open = 0usize;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 1,
                        b'(' => open += 1,
                        b')' => {
                            open -= 1;
                            if open == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // include `)`
                }
                TokenKind::Other
            }
            b'[' => {
                // An array runs to the closing `]`, past any comment.
                while i < bytes.len() && bytes[i] != b']' {
                    i = if bytes[i] == b'%' {
                        trivia(bytes, i)
                    } else {
                        i + 1
                    };
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
                    && !matches!(bytes[i], b'<' | b'[' | b'(' | b'%')
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

    /// An entry missing its destination is not one, so the section still
    /// ends at its end keyword.
    #[test]
    fn an_entry_without_a_destination_leaves_the_section_end() {
        let cmap = b"1 beginbfchar <01> <2F2D> <02> endbfchar <03> <2F2D>";
        let out =
            String::from_utf8(readable_cmap(cmap, &mut { usize::MAX }).unwrap().unwrap()).unwrap();
        assert_eq!(out, "1 beginbfchar <01> <5C71> <02> endbfchar <03> <2F2D>");

        let cmap = b"1 beginbfrange <01> <02> endbfrange 1 beginbfchar <05> <0041> endbfchar";
        let out = strip_codes(cmap, &codes(&[b"\x05"])).unwrap();
        assert_eq!(
            out,
            b"1 beginbfrange <01> <02> endbfrange 1 beginbfchar endbfchar"
        );
    }

    /// A kept range is passed over whole, so its destination is not read as
    /// the start of another range.
    #[test]
    fn a_kept_range_keeps_the_next_aligned() {
        let cmap = b"2 beginbfrange <01> <02> <0005> <05> <05> <0042> endbfrange";
        let out = strip_codes(cmap, &codes(&[b"\x05"])).unwrap();
        assert_eq!(out, b"2 beginbfrange <01> <02> <0005> endbfrange");
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

    /// A range of a few bytes expands to all its destinations, so a rewrite
    /// growing past its budget is refused, and one within it takes its growth.
    #[test]
    fn a_rewrite_takes_its_growth_from_the_budget() {
        let cmap = b"1 beginbfrange\n<0000> <00FF> <2F00>\nendbfrange\n";
        let err = readable_cmap(cmap, &mut 1_000).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ResourceLimit);
        let mut budget = 10_000;
        let out = readable_cmap(cmap, &mut budget).unwrap().unwrap();
        assert_eq!(10_000 - budget, out.len() - cmap.len());
    }

    /// A range expanding past the budget is refused as it expands, before
    /// its destinations are all held.
    #[test]
    fn a_wide_range_is_refused_within_its_budget() {
        let dst = format!("<{}>", "2F00".repeat(8_192));
        let cmap = format!("1 beginbfrange\n<0000> <FFFF> {dst}\nendbfrange\n");
        let err = readable_cmap(cmap.as_bytes(), &mut 0).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ResourceLimit);
    }

    /// An entry in a comment is not one: a viewer never reads it, so it is
    /// neither rewritten nor charged to the budget.
    #[test]
    fn an_entry_in_a_comment_is_left_alone() {
        let cmap = b"1 beginbfrange\n% <0000> <FFFF> <2F00>\n<01> <01> <0041>\nendbfrange\n";
        assert_eq!(readable_cmap(cmap, &mut 0).unwrap(), None);
        let cmap = b"1 beginbfchar % <01> <2F2D>\n<02> <2F2D> endbfchar";
        let out = readable_cmap(cmap, &mut { usize::MAX }).unwrap().unwrap();
        assert_eq!(out, b"1 beginbfchar % <01> <2F2D>\n<02> <5C71> endbfchar");
    }

    /// A comment and a string are whole tokens, so the tokens still
    /// reproduce the input.
    #[test]
    fn tokenize_keeps_comments_and_strings() {
        let cmap = "/Registry (Adobe % not a comment) def % a [comment\n<01> <0041>";
        let tokens = tokenize(cmap);
        let joined: String = tokens.iter().map(|t| t.text).collect();
        assert_eq!(joined, cmap);
        let words: Vec<&str> = tokens.iter().map(Token::word).collect();
        assert_eq!(
            words,
            [
                "/Registry",
                "(Adobe % not a comment)",
                "def",
                "<01>",
                "<0041>"
            ]
        );
    }

    /// A CMap nothing in is rewritten takes nothing from the budget, however
    /// large it is.
    #[test]
    fn an_unchanged_cmap_takes_no_budget() {
        let cmap = b"1 beginbfchar\n<0041> <0041>\nendbfchar\n".repeat(100);
        let mut budget = 0;
        assert_eq!(readable_cmap(&cmap, &mut budget).unwrap(), None);
    }

    #[test]
    fn writes_bfchar_destinations_as_readable_text() {
        // Chrome maps 山 to the Kangxi radical ⼭ (U+2F2D); Quartz maps fi to ﬁ.
        let cmap = b"2 beginbfchar\n<0681> <2F2D>\n<0003> <FB01>\nendbfchar\n";
        let out =
            String::from_utf8(readable_cmap(cmap, &mut { usize::MAX }).unwrap().unwrap()).unwrap();
        assert_eq!(
            out,
            "2 beginbfchar\n<0681> <5C71>\n<0003> <00660069>\nendbfchar\n"
        );
    }

    /// An unchanged entry is passed over whole, so the next one keeps its
    /// own boundaries.
    #[test]
    fn an_unchanged_entry_keeps_the_next_aligned() {
        let cmap = b"2 beginbfchar <0001> <0041> <2F2D> <0042> endbfchar \
            2 beginbfrange <01> <02> <0041> <2F2D> <2F2D> <2F2D> endbfrange";
        let out =
            String::from_utf8(readable_cmap(cmap, &mut { usize::MAX }).unwrap().unwrap()).unwrap();
        assert_eq!(
            out,
            "2 beginbfchar <0001> <0041> <2F2D> <0042> endbfchar \
            2 beginbfrange <01> <02> <0041> <2F2D> <2F2D> [<5C71>] endbfrange"
        );
    }

    #[test]
    fn leaves_a_cmap_without_forms_alone() {
        let cmap = b"1 beginbfchar\n<0041> <0041>\nendbfchar\n1 beginbfrange\n<01> <03> <0061>\nendbfrange\n";
        assert_eq!(readable_cmap(cmap, &mut { usize::MAX }).unwrap(), None);
    }

    #[test]
    fn writes_a_changed_range_as_its_destinations() {
        // <2F2B>..<2F2D> are the radicals for 尸, 屮 and 山.
        let cmap = b"1 beginbfrange\n<10> <12> <2F2B>\nendbfrange\n";
        let out =
            String::from_utf8(readable_cmap(cmap, &mut { usize::MAX }).unwrap().unwrap()).unwrap();
        assert_eq!(
            out,
            "1 beginbfrange\n<10> <12> [<5C38> <5C6E> <5C71>]\nendbfrange\n"
        );
    }

    #[test]
    fn writes_a_range_array_entry_by_entry() {
        let cmap = b"beginbfrange <10> <11> [<0041> <2F2D>] endbfrange";
        let out =
            String::from_utf8(readable_cmap(cmap, &mut { usize::MAX }).unwrap().unwrap()).unwrap();
        assert_eq!(out, "beginbfrange <10> <11> [<0041> <5C71>] endbfrange");
    }

    #[test]
    fn keeps_bytes_outside_the_destinations() {
        let cmap = b"/CMapName /\xE9 def\nbeginbfchar <01> <2F2D> endbfchar";
        let out = readable_cmap(cmap, &mut { usize::MAX }).unwrap().unwrap();
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
