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

use crate::pdf::reader::{DictionaryExt, DocumentExt, MAX_CONTENT_BYTES};

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

/// How a font's codes split into glyphs and read as text (9.10.2).
pub(crate) enum FontText<'a> {
    /// A composite font with a `/ToUnicode` CMap: the CMap splits its codes
    /// and gives their text. Its codespace is the font's encoding's (9.10.3),
    /// so this holds whatever the encoding: `Identity-H`, a predefined CMap,
    /// or one the font embeds.
    Mapped(Encoding<'a>),
    /// A composite font with no `/ToUnicode` under a predefined Unicode CMap
    /// (`UniJIS-UCS2-H`, `UniGB-UTF16-V`, ...): its codes are UTF-16BE.
    Utf16,
    /// A simple font: one byte per code, read through its `/ToUnicode` where
    /// that maps the code, else its encoding.
    Simple {
        encoding: Encoding<'a>,
        to_unicode: Option<Encoding<'static>>,
    },
}

impl<'a> FontText<'a> {
    /// The text decoding of `font`, or `None` when its codes can't be read:
    /// its encoding can't be, or it is a composite font with neither a
    /// `/ToUnicode` lopdf reads nor a Unicode encoding.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// its `/ToUnicode` CMap decodes past the content limit.
    pub(crate) fn of(doc: &'a Document, font: &'a Dictionary) -> Result<Option<Self>> {
        let to_unicode = match font.get(b"ToUnicode").ok().and_then(|c| doc.resolve(c)) {
            Some(Object::Stream(stream)) => to_unicode(doc, stream)?,
            _ => None,
        };
        if font.is_subtype(doc, b"Type0") {
            if let Some(map) = to_unicode {
                return Ok(Some(Self::Mapped(map)));
            }
            let unicode = font.name(doc, b"Encoding").is_some_and(is_unicode_cmap);
            return Ok(unicode.then_some(Self::Utf16));
        }
        let Ok(encoding) = font.get_font_encoding_with_limit(doc, MAX_CONTENT_BYTES) else {
            return Ok(None);
        };
        Ok(Some(match encoding {
            map @ Encoding::UnicodeMapEncoding(_) => Self::Mapped(map),
            encoding => Self::Simple {
                encoding,
                to_unicode,
            },
        }))
    }
}

/// Whether the predefined CMap `name` maps UCS-2 or UTF-16 codes (9.7.5.2).
fn is_unicode_cmap(name: &[u8]) -> bool {
    name.starts_with(b"Uni")
        && [&b"-UCS2-"[..], b"-UTF16-"]
            .iter()
            .any(|form| name.windows(form.len()).any(|window| window == *form))
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
    match font {
        // Consume variable-length codes through the CMap, one glyph per
        // successful lookup, the same greedy consumption the CMap decode uses.
        FontText::Mapped(Encoding::UnicodeMapEncoding(cmap)) => {
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
        FontText::Mapped(_) => Err(Error::redaction(
            "text drawn with a font whose CMap cannot be read; its glyphs cannot be \
             located for redaction",
        )),
        // UTF-16BE: a unit per glyph, a surrogate pair as one.
        FontText::Utf16 => {
            let (units, rest) = bytes.as_chunks::<2>();
            if !rest.is_empty() {
                return Err(Error::redaction(
                    "text drawn in a UTF-16 font ends inside a code; its glyphs cannot \
                     be located for redaction",
                ));
            }
            let mut glyphs = Vec::new();
            let mut i = 0;
            while i < units.len() {
                let unit = u16::from_be_bytes(units[i]);
                let len = if (0xD800..0xDC00).contains(&unit) && i + 1 < units.len() {
                    2
                } else {
                    1
                };
                let codes: Vec<u16> = units[i..i + len]
                    .iter()
                    .map(|&pair| u16::from_be_bytes(pair))
                    .collect();
                glyphs.push(Glyph {
                    byte_start: 2 * i,
                    byte_end: 2 * (i + len),
                    code: codes.iter().fold(0, |code, &u| code << 16 | u32::from(u)),
                    text: mapped_text(&codes)?,
                });
                i += len;
            }
            Ok(glyphs)
        }
        // Simple font: one byte per glyph, read through `/ToUnicode` where it
        // maps the byte, else the encoding; a byte neither decodes is likewise
        // an unmatchable glyph.
        FontText::Simple {
            encoding,
            to_unicode,
        } => bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let mapped = match to_unicode {
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

    /// A one-byte-and-two-byte `/ToUnicode` CMap mapping `<41>` to `J` and
    /// `<8140>` to `a`, as a Shift-JIS font's would.
    const TO_UNICODE: &[u8] = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
        /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
        /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
        2 begincodespacerange\n<00> <80>\n<8140> <9FFC>\nendcodespacerange\n\
        2 beginbfchar\n<41> <004A>\n<8140> <0061>\nendbfchar\n\
        endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";

    /// The glyphs `bytes` draw in a composite font of `encoding`, with the
    /// `/ToUnicode` above when `mapped`.
    fn composite(encoding: Object, mapped: bool, bytes: &[u8]) -> Option<Result<Vec<String>>> {
        let mut doc = Document::with_version("1.7");
        let mut font = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X", "Encoding" => encoding,
        };
        if mapped {
            font.set(
                "ToUnicode",
                doc.add_object(Stream::new(dictionary! {}, TO_UNICODE.to_vec())),
            );
        }
        let text = FontText::of(&doc, &font).unwrap()?;
        Some(
            decode_glyphs(&text, bytes)
                .map(|glyphs| glyphs.into_iter().map(|glyph| glyph.text).collect()),
        )
    }

    /// A composite font's `/ToUnicode` splits and reads its codes whatever
    /// its encoding: a predefined CMap other than `Identity-H`, or one the
    /// font embeds.
    #[test]
    fn a_composite_font_reads_through_its_to_unicode() {
        let embedded = Object::Stream(Stream::new(dictionary! { "Type" => "CMap" }, Vec::new()));
        for encoding in [Object::Name(b"90ms-RKSJ-H".to_vec()), embedded] {
            let glyphs = composite(encoding, true, b"\x41\x81\x40").unwrap().unwrap();
            assert_eq!(glyphs, ["J", "a"]);
        }
    }

    /// A code the `/ToUnicode` does not map has no text to read, so the text
    /// drawn with it is refused rather than read as the encoding's CIDs.
    #[test]
    fn a_code_its_to_unicode_does_not_map_is_refused() {
        let name = Object::Name(b"90ms-RKSJ-H".to_vec());
        let err = composite(name, true, b"\x42").unwrap().unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::Redaction);
    }

    /// A composite font under a predefined Unicode CMap and no `/ToUnicode`
    /// reads its codes as UTF-16, a surrogate pair as one glyph.
    #[test]
    fn a_unicode_cmap_reads_as_utf16() {
        let name = Object::Name(b"UniJIS-UTF16-H".to_vec());
        let glyphs = composite(name, false, b"\x00J\xD8\x3D\xDE\x00")
            .unwrap()
            .unwrap();
        assert_eq!(glyphs, ["J", "\u{1F600}"]);
    }

    /// A composite font with no `/ToUnicode` and an encoding that is not
    /// Unicode has no text to read.
    #[test]
    fn a_composite_font_with_nothing_to_read_is_unreadable() {
        assert!(composite(Object::Name(b"Identity-H".to_vec()), false, b"\x00\x01").is_none());
    }

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
