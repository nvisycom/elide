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
pub(super) fn decode_glyphs(encoding: &Encoding, bytes: &[u8]) -> Vec<Glyph> {
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
                        text = Some(String::from_utf16_lossy(&utf16));
                        consumed = len;
                        break;
                    }
                }
                // A code with no mapping is consumed as one byte with empty text,
                // keeping byte offsets exact.
                let consumed = consumed.max(1);
                glyphs.push(Glyph {
                    byte_start: i,
                    byte_end: i + consumed,
                    text: text.unwrap_or_default(),
                });
                i += consumed;
            }
            glyphs
        }
        // Simple font: one byte per glyph. Decode each byte on its own.
        _ => bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| Glyph {
                byte_start: i,
                byte_end: i + 1,
                text: encoding.bytes_to_string(&[b]).unwrap_or_default(),
            })
            .collect(),
    }
}
