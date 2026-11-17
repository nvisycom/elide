//! Per-glyph decoding: split a text-drawing operand's bytes into individual
//! glyphs, each with its byte range and decoded text.
//!
//! A detected character span must map to *exact* glyph byte ranges so deletion
//! removes whole glyph codes, never a partial code. Decoding matches how the PDF
//! text machinery consumes codes: a composite (Type0) font's `/ToUnicode` CMap
//! resolves variable-length codes (1–4 bytes) one at a time, while a simple font
//! decodes one byte per glyph. A glyph's text is what a viewer copies: its
//! code's `/ToUnicode` mapping, else its encoding's character (9.10.2).

use std::iter;

use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Encoding, Error as PdfError, Object, Stream, dictionary};
use unicode_normalization::UnicodeNormalization;

use crate::pdf::reader::{DocumentExt, MAX_CONTENT_BYTES};

/// One decoded glyph within an operand's bytes.
pub(crate) struct Glyph {
    /// Start of the glyph's code within the operand string.
    pub(crate) byte_start: usize,
    /// End of the glyph's code (exclusive).
    pub(crate) byte_end: usize,
    /// The glyph's raw character code (the bytes consumed, big-endian). For a
    /// composite font under an `Identity` encoding this is the CID, so it keys the
    /// font's `/W` width table; for a simple font it is the single byte.
    pub(crate) code: u32,
    /// The text this glyph decodes to (usually one char; a ligature is several).
    pub(crate) text: String,
}

/// The text detection reads for a glyph's decoded `text`: each character a
/// font's glyph stands for in a form of its own replaced by the logical text
/// it writes, every other character as it is.
///
/// A font often draws a radical and its ideograph, or a ligature and the
/// letters it joins, with one glyph, and a producer's `/ToUnicode` may then map
/// the glyph to the form rather than the text (Chrome maps `山` to the radical
/// `⼭` and supplies the ideograph only as `/ActualText`; Quartz maps `fi` to
/// `ﬁ`): `山田` reads `⼭⽥`, `file` reads `ﬁle`, and nothing that matches the
/// text sees it. The forms replaced are the Kangxi radicals and Unicode's
/// presentation forms (Latin, Armenian, Hebrew, Arabic), by their compatibility
/// decomposition, and the CJK compatibility ideographs, by their canonical
/// one. A glyph may stand for several characters, so a replacement of another
/// length leaves the glyph map intact.
pub(crate) fn readable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match u32::from(c) {
            0x2F00..=0x2FDF | 0xFB00..=0xFDFF | 0xFE70..=0xFEFF => {
                out.extend(iter::once(c).nfkc());
            }
            0xF900..=0xFAFF | 0x2F800..=0x2FA1F => out.extend(iter::once(c).nfc()),
            _ => out.push(c),
        }
    }
    out
}

/// How a font's codes read as text: the encoding that splits them into
/// glyphs, and, for a simple font, its `/ToUnicode` CMap, which gives a code's
/// text ahead of the encoding. (A composite font's encoding is its CMap.)
pub(crate) struct FontText<'a> {
    encoding: Encoding<'a>,
    to_unicode: Option<Encoding<'static>>,
}

impl<'a> FontText<'a> {
    /// The text decoding of `font`, or `None` when its encoding can't be read.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// its `/ToUnicode` CMap decodes past the content limit.
    pub(crate) fn of(doc: &'a Document, font: &'a Dictionary) -> Result<Option<Self>> {
        let Ok(encoding) = font.get_font_encoding_with_limit(doc, MAX_CONTENT_BYTES) else {
            return Ok(None);
        };
        let to_unicode = match (&encoding, font.get(b"ToUnicode")) {
            (Encoding::UnicodeMapEncoding(_), _) | (_, Err(_)) => None,
            (_, Ok(cmap)) => match doc.resolve(cmap) {
                Some(Object::Stream(stream)) => to_unicode(doc, stream)?,
                _ => None,
            },
        };
        Ok(Some(Self {
            encoding,
            to_unicode,
        }))
    }
}

/// `stream` read as a `/ToUnicode` CMap by lopdf, the reader detection
/// decodes text with; `None` when lopdf can't read it.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if it
/// decodes past the content limit.
pub(crate) fn to_unicode(doc: &Document, stream: &Stream) -> Result<Option<Encoding<'static>>> {
    // lopdf parses a CMap only as a font's: a font with no `/Encoding` reads
    // its `/ToUnicode`, and falls back to a one-byte encoding when it can't.
    let font = dictionary! { "Type" => "Font", "ToUnicode" => Object::Stream(stream.clone()) };
    match font.get_font_encoding_with_limit(doc, MAX_CONTENT_BYTES) {
        Ok(Encoding::UnicodeMapEncoding(cmap)) => Ok(Some(Encoding::UnicodeMapEncoding(cmap))),
        Err(PdfError::Decompress(e)) => Err(Error::resource_limit(format!(
            "a /ToUnicode CMap decodes past the content limit: {e}"
        ))),
        _ => Ok(None),
    }
}

/// The text of a glyph whose code maps to the UTF-16 `units`.
///
/// Fails closed on units that are not UTF-16 (an unpaired surrogate): a lossy
/// `�` would misrepresent the source text and could make a PII character
/// undetectable.
fn mapped_text(units: &[u16]) -> Result<String> {
    let decoded = String::from_utf16(units).map_err(|_| {
        Error::redaction(
            "text drawn with a code whose CMap mapping is not valid UTF-16; \
             its glyph cannot be reliably decoded for redaction",
        )
    })?;
    Ok(readable(&decoded))
}

/// Split `bytes` into glyphs under `font`.
///
/// Fails closed: a code that does not decode under the font is an unmatchable
/// glyph, it could not be located for deletion, so the whole redaction is
/// refused rather than leaving that text silently in place.
pub(crate) fn decode_glyphs(font: &FontText<'_>, bytes: &[u8]) -> Result<Vec<Glyph>> {
    match &font.encoding {
        // Composite font: consume variable-length codes through the CMap, one
        // glyph per successful lookup, the same greedy consumption the CMap
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
                        text = Some(mapped_text(&utf16)?);
                        consumed = len;
                        break;
                    }
                }
                let Some(text) = text else {
                    return Err(Error::redaction(
                        "text drawn with a code that has no CMap mapping; \
                         its glyph cannot be located for redaction",
                    ));
                };
                glyphs.push(Glyph {
                    byte_start: i,
                    byte_end: i + consumed,
                    code,
                    text,
                });
                i += consumed;
            }
            Ok(glyphs)
        }
        // Simple font: one byte per glyph, read through `/ToUnicode` where it
        // maps the byte, else the encoding; a byte neither decodes is likewise
        // an unmatchable glyph.
        encoding => bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let mapped = match &font.to_unicode {
                    Some(Encoding::UnicodeMapEncoding(cmap)) => cmap.get(u32::from(b), 1),
                    _ => None,
                };
                let text = match mapped {
                    Some(units) => mapped_text(&units)?,
                    None => encoding
                        .bytes_to_string(&[b])
                        .map(|t| readable(&t))
                        .map_err(|_| {
                            Error::redaction(
                                "text drawn with a byte that does not decode under its \
                                 font; its glyph cannot be located for redaction",
                            )
                        })?,
                };
                Ok(Glyph {
                    byte_start: i,
                    byte_end: i + 1,
                    code: u32::from(b),
                    text,
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_forms_read_as_their_text() {
        assert_eq!(readable("⼭⽥太郎 ⽇本 中⽂"), "山田太郎 日本 中文");
        assert_eq!(readable("\u{F900}"), "\u{8C48}");
        assert_eq!(readable("ﬁle ﬀ"), "file ff");
        // Arabic presentation forms read as their letters.
        assert_eq!(readable("\u{FE8D}\u{FEDF}"), "\u{0627}\u{0644}");
        // Other text, fullwidth forms included, is left as written.
        assert_eq!(readable("Jane １２３"), "Jane １２３");
    }
}
