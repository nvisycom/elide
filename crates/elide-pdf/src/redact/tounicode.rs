//! Scrub deleted glyph codes from a font's `/ToUnicode` CMap.
//!
//! A `/ToUnicode` CMap maps character codes to Unicode, so text can be recovered
//! from it (copy, search, extraction) independently of the drawn glyphs. When
//! [`redact_text`](crate::Pdf::redact_text) deletes a glyph, its code must also
//! be removed from this table, or the redacted text survives as a code->Unicode
//! entry. The CMap is edited textually: a `bfchar` entry for a deleted code is
//! dropped, and a `bfrange` covering one fails closed (its interior can't be
//! partially removed without re-deriving the range).

use std::collections::{BTreeMap, BTreeSet};

use lopdf::{Document, Object, ObjectId};

use crate::error::{Error, Result};

/// Map a page's font resource names to the object id of each font's `/ToUnicode`
/// stream, for fonts that have one (both simple and Type0 fonts may).
pub(super) fn page_font_cmaps(doc: &Document, page_id: ObjectId) -> BTreeMap<Vec<u8>, ObjectId> {
    let mut out = BTreeMap::new();
    let Ok(fonts) = doc.get_page_fonts(page_id) else {
        return out;
    };
    for (name, font) in fonts {
        if let Ok(to_unicode) = font.get(b"ToUnicode")
            && let Ok(id) = to_unicode.as_reference()
        {
            out.insert(name, id);
        }
    }
    out
}

/// Remove `deleted` codes from each named `/ToUnicode` CMap stream.
///
/// # Errors
///
/// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a deleted
/// code falls inside a multi-code `bfrange` (which cannot be scrubbed without
/// re-deriving the range) or if a CMap stream cannot be read, so the redaction
/// fails closed rather than leaving a recoverable code->Unicode entry.
pub(super) fn scrub(
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
            .map_err(|e| Error::unsafe_rewrite(format!("read /ToUnicode CMap: {e}")))?;
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
fn strip_codes(cmap: &[u8], codes: &BTreeSet<Vec<u8>>) -> Result<Vec<u8>> {
    let text = String::from_utf8_lossy(cmap);
    let mut out = String::with_capacity(text.len());
    let mut in_bfchar = false;
    let mut in_bfrange = false;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.ends_with("beginbfchar") {
            in_bfchar = true;
            out.push_str(line);
            continue;
        }
        if trimmed.ends_with("beginbfrange") {
            in_bfrange = true;
            out.push_str(line);
            continue;
        }
        if trimmed == "endbfchar" {
            in_bfchar = false;
            out.push_str(line);
            continue;
        }
        if trimmed == "endbfrange" {
            in_bfrange = false;
            out.push_str(line);
            continue;
        }

        if in_bfchar {
            // `<src> <dst>` — drop the line when `<src>` is a deleted code.
            if let Some(src) = first_hex_code(trimmed)
                && codes.contains(&src)
            {
                continue;
            }
        } else if in_bfrange {
            // `<lo> <hi> <dst>` — a deleted code inside a genuine range can't be
            // excised textually; fail closed. A degenerate range (lo == hi) that
            // equals a deleted code is safe to drop.
            let hexes = hex_codes(trimmed);
            if let (Some(lo), Some(hi)) = (hexes.first(), hexes.get(1)) {
                let hit = codes.iter().any(|c| c >= lo && c <= hi);
                if hit {
                    if lo == hi && codes.contains(lo) {
                        continue;
                    }
                    return Err(Error::unsafe_rewrite(
                        "a deleted glyph's code falls inside a /ToUnicode bfrange; \
                         the code->Unicode mapping cannot be scrubbed safely",
                    ));
                }
            }
        }
        out.push_str(line);
    }
    Ok(out.into_bytes())
}

/// The first `<hex>` token on a line, as raw bytes (e.g. `<0041>` -> `[0x00,0x41]`).
fn first_hex_code(line: &str) -> Option<Vec<u8>> {
    hex_codes(line).into_iter().next()
}

/// Every `<hex>` token on a line, each decoded to raw bytes.
fn hex_codes(line: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            break;
        };
        if let Some(bytes) = hex_to_bytes(&after[..close]) {
            out.push(bytes);
        }
        rest = &after[close + 1..];
    }
    out
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
        assert_eq!(err.kind(), crate::ErrorKind::UnsafeRewrite);
    }

    #[test]
    fn hex_decodes() {
        assert_eq!(hex_to_bytes("0041"), Some(vec![0x00, 0x41]));
        assert_eq!(hex_to_bytes("41"), Some(vec![0x41]));
        assert_eq!(hex_to_bytes("XYZ"), None);
        assert_eq!(hex_to_bytes("041"), None);
    }
}
