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
use std::sync::LazyLock;

use elide_core::{Error, ErrorKind, Result};
use lopdf::{Dictionary, Document, Encoding, Object, Stream, dictionary};
use unicode_normalization::UnicodeNormalization;

use crate::pdf::reader::{DictionaryExt, DocumentExt, MAX_CONTENT_BYTES, StreamExt};

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
///
/// A text that would read past what a CMap destination holds (256 UTF-16
/// units) stays as written, so the text detection reads is the text a CMap
/// can be written with.
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
    if out.encode_utf16().count() > MAX_DESTINATION_UNITS {
        return text.to_owned();
    }
    out
}

/// The most UTF-16 units a CMap destination holds (9.10.3).
const MAX_DESTINATION_UNITS: usize = 256;

/// How a font's codes split into glyphs and read as text (9.10.2).
pub(crate) enum FontText<'a> {
    /// A composite (Type0) font: codes of the lengths its codespace gives
    /// them, each read through its `/ToUnicode`, else, under a Unicode
    /// encoding, as the UTF-16 it is.
    Composite {
        /// The length of a code, by its first byte; 0 for a byte no code
        /// starts with.
        lengths: Box<[u8; 256]>,
        to_unicode: Option<Encoding<'static>>,
        /// Whether the encoding is a predefined Unicode CMap, its codes the
        /// UTF-16 of their text.
        unicode: bool,
    },
    /// A simple font: one byte per code, read through its `/ToUnicode` where
    /// that maps the code, else its encoding.
    Simple {
        encoding: Encoding<'a>,
        to_unicode: Option<Encoding<'static>>,
    },
}

impl<'a> FontText<'a> {
    /// The text decoding of `font`, or `None` when its codes can't be read:
    /// a simple font's encoding can't be, or a composite font's code
    /// boundaries can't be established (see [`composite_lengths`]).
    ///
    /// # Errors
    ///
    /// As [`to_unicode`].
    pub(crate) fn of(doc: &'a Document, font: &'a Dictionary) -> Result<Option<Self>> {
        let to_unicode = match font.get(b"ToUnicode").ok().and_then(|c| doc.resolve(c)) {
            Some(Object::Stream(stream)) => to_unicode(doc, stream)?,
            _ => None,
        };
        if font.is_subtype(doc, b"Type0") {
            let encoding = font.name(doc, b"Encoding");
            let unicode = encoding.is_some_and(is_unicode_cmap);
            let Some(lengths) = composite_lengths(encoding, to_unicode.as_ref()) else {
                return Ok(None);
            };
            return Ok(Some(Self::Composite {
                lengths,
                to_unicode,
                unicode,
            }));
        }
        let Ok(encoding) = font.get_font_encoding_with_limit(doc, MAX_CONTENT_BYTES) else {
            return Ok(None);
        };
        // A simple font with no `/Encoding` is read by lopdf through its
        // `/ToUnicode` alone; its codes are still one byte, and one the map
        // lacks reads through the standard encoding.
        let encoding = match encoding {
            Encoding::UnicodeMapEncoding(_) => standard(),
            encoding => encoding,
        };
        Ok(Some(Self::Simple {
            encoding,
            to_unicode,
        }))
    }
}

/// The lengths of a composite font's codes by their first byte: two bytes
/// under `Identity-H`/`-V` and a UCS-2 CMap, two or four (a surrogate pair)
/// under a UTF-16 one; under any other encoding, the lengths of the codes its
/// `/ToUnicode` maps, whose codespace is the encoding's (9.10.3). `None` when
/// they can't be established: no `/ToUnicode`, or one whose codes of
/// different lengths start with the same byte.
fn composite_lengths(
    encoding: Option<&[u8]>,
    to_unicode: Option<&Encoding<'_>>,
) -> Option<Box<[u8; 256]>> {
    let mut lengths = Box::new([0u8; 256]);
    match encoding {
        Some(b"Identity-H" | b"Identity-V") => *lengths = [2; 256],
        Some(name) if is_unicode_cmap(name) => {
            *lengths = [2; 256];
            if contains(name, b"-UTF16-") {
                lengths[0xD8..=0xDB].fill(4);
            }
        }
        _ => {
            let Some(Encoding::UnicodeMapEncoding(cmap)) = to_unicode else {
                return None;
            };
            for (len, ranges) in (1u8..).zip(&cmap.bf_ranges) {
                let shift = 8 * (u32::from(len) - 1);
                for (range, _) in ranges.iter() {
                    for lead in (range.start() >> shift)..=(range.end() >> shift) {
                        let slot = &mut lengths[(lead & 0xFF) as usize];
                        if *slot != 0 && *slot != len {
                            return None;
                        }
                        *slot = len;
                    }
                }
            }
        }
    }
    Some(lengths)
}

/// The standard encoding, which lopdf reads a font with no other by.
fn standard() -> Encoding<'static> {
    static PLAIN: LazyLock<(Document, Dictionary)> =
        LazyLock::new(|| (Document::new(), dictionary! { "Type" => "Font" }));
    let (doc, font) = &*PLAIN;
    font.get_font_encoding(doc)
        .unwrap_or(Encoding::SimpleEncoding(b"StandardEncoding"))
}

/// Whether the predefined CMap `name` maps UCS-2 or UTF-16 codes (9.7.5.2).
fn is_unicode_cmap(name: &[u8]) -> bool {
    name.starts_with(b"Uni") && (contains(name, b"-UCS2-") || contains(name, b"-UTF16-"))
}

/// Whether `name` holds `part`.
fn contains(name: &[u8], part: &[u8]) -> bool {
    name.windows(part.len()).any(|window| window == part)
}

/// The most codes a `/ToUnicode` CMap's ranges may span. lopdf expands every
/// code of every range as it parses, so a range of four-byte codes could
/// name billions.
const MAX_CODES: u64 = 1 << 18;

/// `stream` read as a `/ToUnicode` CMap by lopdf, the reader detection
/// decodes text with; `None` when it can't be decoded or lopdf can't read it.
///
/// lopdf is given the very bytes whose ranges were counted, unfiltered, so
/// nothing it reads went uncounted.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if it
/// decodes past the content limit, or its ranges span more than
/// [`MAX_CODES`] codes, counted before lopdf expands them.
pub(crate) fn to_unicode(doc: &Document, stream: &Stream) -> Result<Option<Encoding<'static>>> {
    let content = match stream.decoded() {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::ResourceLimit => return Err(e),
        Err(_) => return Ok(None),
    };
    if range_codes(&content) > MAX_CODES {
        return Err(Error::resource_limit(format!(
            "a /ToUnicode CMap's ranges span more than {MAX_CODES} codes"
        )));
    }
    // lopdf parses a CMap only as a font's: a font with no `/Encoding` reads
    // its `/ToUnicode`, and falls back to a one-byte encoding when it can't.
    let plain = Stream::new(Dictionary::new(), content);
    let font = dictionary! { "Type" => "Font", "ToUnicode" => Object::Stream(plain) };
    match font.get_font_encoding(doc) {
        Ok(Encoding::UnicodeMapEncoding(cmap)) => Ok(Some(Encoding::UnicodeMapEncoding(cmap))),
        _ => Ok(None),
    }
}

/// How many codes the `bfrange` entries of the CMap `content` span, at
/// least: each `<lo> <hi>` pair of hex strings of one length opening an
/// entry. Comments are skipped, and strings and dictionary delimiters are
/// passed over whole, so a `%` or `>` inside them hides nothing.
fn range_codes(content: &[u8]) -> u64 {
    let mut tokens = Vec::new();
    let mut in_ranges = false;
    let mut i = 0;
    while i < content.len() {
        match content[i] {
            b'%' => {
                while i < content.len() && !matches!(content[i], b'\n' | b'\r') {
                    i += 1;
                }
            }
            b'(' => i = past_string(content, i),
            b'<' | b'>' if content.get(i + 1) == Some(&content[i]) => i += 2,
            b'<' => {
                let end = content[i..]
                    .iter()
                    .position(|&b| b == b'>')
                    .map_or(content.len(), |e| i + e);
                if in_ranges {
                    tokens.push(Some(&content[i + 1..end]));
                }
                i = end + 1;
            }
            b'[' => {
                i += content[i..]
                    .iter()
                    .position(|&b| b == b']')
                    .unwrap_or(content.len() - i)
                    + 1;
                if in_ranges {
                    tokens.push(None);
                }
            }
            b if b.is_ascii_alphabetic() => {
                let start = i;
                while i < content.len() && content[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                match &content[start..i] {
                    b"beginbfrange" => in_ranges = true,
                    b"endbfrange" => in_ranges = false,
                    _ => {}
                }
            }
            _ => i += 1,
        }
    }
    let code = |hex: &[u8]| {
        let digits: Vec<u8> = hex
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        (digits.len() <= 8 && digits.len().is_multiple_of(2))
            .then(|| u64::from_str_radix(std::str::from_utf8(&digits).ok()?, 16).ok())
            .flatten()
            .map(|value| (digits.len(), value))
    };
    let mut count = 0u64;
    let mut i = 0;
    while i + 1 < tokens.len() {
        if let (Some(Some((lo_len, lo))), Some(Some((hi_len, hi)))) =
            (tokens[i].map(code), tokens[i + 1].map(code))
            && lo_len == hi_len
            && hi >= lo
        {
            count = count.saturating_add(hi - lo + 1);
            i += 3;
        } else {
            i += 1;
        }
    }
    count
}

/// The index past the literal string opening at `start` in `content`: its
/// balancing `)`, past escaped bytes and nested parentheses.
fn past_string(content: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i < content.len() {
        match content[i] {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    content.len()
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
        // Codes of the lengths their first bytes give them, each read
        // through `/ToUnicode`, else as UTF-16 under a Unicode encoding.
        FontText::Composite {
            lengths,
            to_unicode,
            unicode,
        } => {
            let mut glyphs = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                let len = usize::from(lengths[usize::from(bytes[i])]);
                let Some(code_bytes) = bytes.get(i..i + len).filter(|_| len > 0) else {
                    return Err(Error::redaction(
                        "text drawn with a code outside its font's codespace; its glyph \
                         cannot be located for redaction",
                    ));
                };
                let code = code_bytes
                    .iter()
                    .fold(0, |code, &b| code << 8 | u32::from(b));
                let mapped = match to_unicode {
                    Some(Encoding::UnicodeMapEncoding(cmap)) => cmap.get(code, len as u8),
                    _ => None,
                };
                let units = match mapped {
                    Some(units) => units,
                    None if *unicode => code_bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&pair| u16::from_be_bytes(pair))
                        .collect(),
                    None => {
                        return Err(Error::redaction(
                            "text drawn with a code that has no CMap mapping; its glyph \
                             cannot be located for redaction",
                        ));
                    }
                };
                glyphs.push(Glyph {
                    byte_start: i,
                    byte_end: i + len,
                    code,
                    text: mapped_text(&units)?,
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

    /// Ranges are counted before lopdf expands them: a CMap spanning too
    /// many codes is refused, and one in a comment is no range.
    #[test]
    fn ranges_are_counted_before_they_are_read() {
        let wide = b"1 beginbfrange\n<000000> <04FFFF> <0041>\nendbfrange\n";
        assert!(range_codes(wide) > MAX_CODES);
        let commented =
            b"1 beginbfrange\n% <000000> <04FFFF> <0041>\n<01> <02> <0041>\nendbfrange\n";
        assert_eq!(range_codes(commented), 2);
        let array = b"1 beginbfrange\n<01> <02> [<0041> <0042>]\n<10> <10> <0043>\nendbfrange\n";
        assert_eq!(range_codes(array), 3);
    }

    /// A `%` inside a string, or a `>` inside a dictionary, hides no range
    /// from the count.
    #[test]
    fn strings_and_dictionaries_hide_no_range() {
        let wide = b"/CIDSystemInfo << /Registry (%) /Ordering (a>b) >> def \
            1 beginbfrange <000000> <04FFFF> <0041> endbfrange";
        assert!(range_codes(wide) > MAX_CODES);
    }

    /// A CMap whose filter is not one is not read at all, so lopdf never
    /// parses bytes that went uncounted.
    #[test]
    fn a_cmap_under_a_filter_that_is_none_is_unreadable() {
        let stream = Stream::new(dictionary! { "Filter" => 5 }, TO_UNICODE.to_vec());
        assert!(to_unicode(&Document::new(), &stream).unwrap().is_none());
    }

    /// A composite font with no `/ToUnicode` and an encoding that is not
    /// Unicode has no text to read: under `Identity-H` its codes split but
    /// read as nothing, and under any other encoding they don't split.
    #[test]
    fn a_composite_font_with_nothing_to_read_is_refused() {
        let identity = composite(Object::Name(b"Identity-H".to_vec()), false, b"\x00\x01");
        assert!(identity.unwrap().is_err());
        assert!(composite(Object::Name(b"90ms-RKSJ-H".to_vec()), false, b"\x41").is_none());
    }

    /// Under a Unicode encoding, a code the `/ToUnicode` does not map reads
    /// as the UTF-16 it is.
    #[test]
    fn a_unicode_encoding_reads_a_code_its_to_unicode_lacks() {
        let name = Object::Name(b"UniJIS-UCS2-H".to_vec());
        let glyphs = composite(name, true, b"\x81\x40\x00\x62").unwrap().unwrap();
        assert_eq!(glyphs, ["a", "b"]);
    }

    /// Codes split by the encoding, not by what the `/ToUnicode` maps: under
    /// `Identity-H` a one-byte mapping never shortens a two-byte code.
    #[test]
    fn identity_codes_are_two_bytes_whatever_the_map_holds() {
        let name = Object::Name(b"Identity-H".to_vec());
        let err = composite(name, true, b"\x41\x81").unwrap().unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::Redaction);
    }

    /// A simple font with no `/Encoding` reads a byte its `/ToUnicode` lacks
    /// through the standard encoding, still one byte a code.
    #[test]
    fn a_simple_font_without_an_encoding_falls_back_per_byte() {
        let mut doc = Document::with_version("1.7");
        let map = doc.add_object(Stream::new(dictionary! {}, TO_UNICODE.to_vec()));
        let font = dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => map,
        };
        let text = FontText::of(&doc, &font).unwrap().unwrap();
        let glyphs: Vec<String> = decode_glyphs(&text, b"\x41\x42")
            .unwrap()
            .into_iter()
            .map(|glyph| glyph.text)
            .collect();
        assert_eq!(glyphs, ["J", "B"]);
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
