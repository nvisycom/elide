//! Per-glyph decoding: split a text-drawing operand's bytes into individual
//! glyphs, each with its byte range and decoded text.
//!
//! A detected character span must map to *exact* glyph byte ranges so deletion
//! removes whole glyph codes, never a partial code. Decoding matches how the PDF
//! text machinery consumes codes: a composite (Type0) font's `/ToUnicode` CMap
//! resolves variable-length codes (1–4 bytes) one at a time, while a simple font
//! decodes one byte per glyph. This mirrors lopdf's own text extraction, so a
//! character offset lines up with the glyph that drew it.

use lopdf::Encoding;

use crate::error::{Error, Result};

/// One decoded glyph within an operand's bytes.
pub(super) struct Glyph {
    /// Start of the glyph's code within the operand string.
    pub(super) byte_start: usize,
    /// End of the glyph's code (exclusive).
    pub(super) byte_end: usize,
    /// The text this glyph decodes to (usually one char; a ligature is several).
    pub(super) text: String,
}

/// Split `bytes` into glyphs under `encoding`.
///
/// Fails closed: a code that does not decode under the font's encoding is an
/// unmatchable glyph — it could not be located for deletion, so the whole
/// redaction is refused rather than leaving that text silently in place.
pub(super) fn decode_glyphs(encoding: &Encoding, bytes: &[u8]) -> Result<Vec<Glyph>> {
    match encoding {
        // Composite font: consume variable-length codes through the CMap, one
        // glyph per successful lookup — the same greedy consumption the CMap
        // decode uses.
        Encoding::UnicodeMapEncoding(cmap) => {
            let mut glyphs = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                let mut code: u32 = 0;
                let mut consumed = 0;
                let mut text = None;
                for len in 1..=4usize {
                    if i + len > bytes.len() {
                        break;
                    }
                    code = (code << 8) | bytes[i + len - 1] as u32;
                    if let Some(utf16) = cmap.get(code, len as u8) {
                        // Fail closed on malformed CMap data (e.g. an unpaired
                        // surrogate): a lossy `�` would misrepresent the source
                        // text and could make a PII character undetectable.
                        let decoded = String::from_utf16(&utf16).map_err(|_| {
                            Error::unsafe_rewrite(
                                "text drawn with a code whose CMap mapping is not \
                                 valid UTF-16; its glyph cannot be reliably decoded \
                                 for redaction",
                            )
                        })?;
                        text = Some(decoded);
                        consumed = len;
                        break;
                    }
                }
                let Some(text) = text else {
                    return Err(Error::unsafe_rewrite(
                        "text drawn with a code that has no CMap mapping; \
                         its glyph cannot be located for redaction",
                    ));
                };
                glyphs.push(Glyph {
                    byte_start: i,
                    byte_end: i + consumed,
                    text,
                });
                i += consumed;
            }
            Ok(glyphs)
        }
        // Simple font: one byte per glyph. Decode each byte on its own; a byte
        // that does not decode is likewise an unmatchable glyph.
        _ => bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let text = encoding.bytes_to_string(&[b]).map_err(|_| {
                    Error::unsafe_rewrite(
                        "text drawn with a byte that does not decode under its font; \
                         its glyph cannot be located for redaction",
                    )
                })?;
                Ok(Glyph {
                    byte_start: i,
                    byte_end: i + 1,
                    text,
                })
            })
            .collect(),
    }
}
