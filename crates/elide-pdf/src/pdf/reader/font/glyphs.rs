//! Per-glyph decoding: split a text-drawing operand's bytes into individual
//! glyphs, each with its byte range and decoded text.
//!
//! A detected character span must map to *exact* glyph byte ranges so deletion
//! removes whole glyph codes, never a partial code. Decoding matches how the PDF
//! text machinery consumes codes: a composite (Type0) font's encoding CMap
//! splits variable-length codes (1–4 bytes), read with `hayro-cmap`, which
//! carries the predefined CMaps; a simple font decodes one byte per glyph. A
//! glyph's text is what a viewer copies: its code's `/ToUnicode` mapping, else
//! its encoding's character (9.10.2).

use std::iter;

use elide_core::{Error, ErrorKind, Result};
use hayro_cmap::{BfString, CMap, CMapName, CidFamily, load_embedded};
use hayro_syntax::object::{Array, Dict, Object, Stream};
use pdf_encoding::{ForwardMap, glyphname_to_unicode};
use unicode_normalization::UnicodeNormalization;

use super::codespace::{Codespace, mapped};
use crate::pdf::reader::{DictExt, StreamExt};

/// One decoded glyph within an operand's bytes.
pub(crate) struct Glyph {
    /// Start of the glyph's code within the operand string.
    pub(crate) byte_start: usize,
    /// End of the glyph's code (exclusive).
    pub(crate) byte_end: usize,
    /// The glyph's raw character code (the bytes consumed, big-endian), which
    /// keys its `/ToUnicode` entry.
    pub(crate) code: u32,
    /// What keys the glyph's width: for a composite font the CID its
    /// encoding maps the code to (0, the missing glyph, when it maps none);
    /// for a simple font the code itself.
    pub(crate) width_key: u32,
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
pub(crate) enum FontText {
    /// A composite (Type0) font, which reads its codes as [`Composite`]
    /// describes.
    Composite(Box<Composite>),
    /// A simple font, which reads its codes as [`Simple`] describes.
    Simple(Box<Simple>),
}

/// How a simple font's codes read: one byte per code, read through its
/// `/ToUnicode` where that maps the code, else its encoding.
pub(crate) struct Simple {
    /// The text of each code under the encoding, or `None` for a font whose
    /// encoding is its font program's own, which is not read.
    encoding: Option<Box<[Option<String>; 256]>>,
    to_unicode: Option<CMap>,
}

/// How a composite (Type0) font's codes read: split by its encoding CMap,
/// each read through its `/ToUnicode`, else, under a Unicode encoding, as
/// the UTF-16 it is, else as its CID's text in the font's character
/// collection.
pub(crate) struct Composite {
    /// The encoding: `Identity-H`/`-V`, a predefined CMap, or one the font
    /// embeds.
    encoding: CMap,
    /// The encoding's codespace, which splits a string into its codes.
    codespace: Codespace,
    to_unicode: Option<CMap>,
    /// Whether the encoding is a predefined Unicode CMap, its codes the
    /// UTF-16 of their text.
    unicode: bool,
    /// The text of each CID in the font's character collection (Adobe's
    /// `-UCS2` CMap for it), when the collection has one.
    collection: Option<CMap>,
}

impl FontText {
    /// The text decoding of `font`, or `None` when its codes can't be read:
    /// a simple font's encoding can't be, or a composite font's encoding
    /// CMap can't be read (see [`encoding_cmap`]).
    ///
    /// # Errors
    ///
    /// As [`to_unicode`].
    pub(crate) fn of(font: &Dict<'_>) -> Result<Option<Self>> {
        let to_unicode = match font.get::<Stream<'_>>(b"ToUnicode") {
            Some(stream) => to_unicode(&stream)?,
            None => None,
        };
        if font.is_subtype(b"Type0") {
            let unicode = font
                .name(b"Encoding")
                .is_some_and(|name| is_unicode_cmap(&name));
            let Some((encoding, codespace)) = encoding_cmap(font)? else {
                return Ok(None);
            };
            let collection = collection_text(font, &encoding);
            return Ok(Some(Self::Composite(Box::new(Composite {
                encoding,
                codespace,
                to_unicode,
                unicode,
                collection,
            }))));
        }
        let encoding = if font.contains_key(b"Encoding") {
            match simple_encoding(font) {
                Some(encoding) => Some(encoding),
                None => return Ok(None),
            }
        } else {
            // With no `/Encoding`, a code the `/ToUnicode` lacks reads through
            // the font's built-in encoding (9.6.5).
            built_in(font).map(table)
        };
        Ok(Some(Self::Simple(Box::new(Simple {
            encoding,
            to_unicode,
        }))))
    }
}

/// The text of each code under the simple `font`'s `/Encoding`: a base
/// encoding by name, or a dictionary's `/BaseEncoding` (else the font's
/// built-in one) with its `/Differences` over it; `None` when it names an
/// encoding not known, a composite one among them, or a built-in one not
/// read.
fn simple_encoding(font: &Dict<'_>) -> Option<Box<[Option<String>; 256]>> {
    if let Some(name) = font.name(b"Encoding") {
        return base_encoding(&name).map(table);
    }
    let dict = font.get::<Dict<'_>>(b"Encoding")?;
    let base = match dict.name(b"BaseEncoding") {
        Some(name) => base_encoding(&name)?,
        None => built_in(font)?,
    };
    let mut codes = table(base);
    if let Some(differences) = dict.get::<Array<'_>>(b"Differences") {
        let mut code: Option<usize> = None;
        for item in differences.iter::<Object<'_>>() {
            match item {
                Object::Number(number) => {
                    code = usize::try_from(number.as_i64()).ok().filter(|&c| c < 256);
                }
                Object::Name(name) => {
                    if let Some(at) = code {
                        codes[at] = glyph_text(&name);
                        code = Some(at + 1).filter(|&c| c < 256);
                    }
                }
                _ => return None,
            }
        }
    }
    Some(codes)
}

/// The base encoding `name` names (Annex D), `None` for one not known.
fn base_encoding(name: &[u8]) -> Option<&'static ForwardMap> {
    match name {
        b"StandardEncoding" => Some(&pdf_encoding::STANDARD),
        b"WinAnsiEncoding" => Some(&pdf_encoding::WINANSI),
        b"MacRomanEncoding" => Some(&pdf_encoding::MACROMAN),
        b"MacExpertEncoding" => Some(&pdf_encoding::MACEXPERT),
        _ => None,
    }
}

/// The text of each code under the base encoding `map`.
fn table(map: &'static ForwardMap) -> Box<[Option<String>; 256]> {
    Box::new(std::array::from_fn(|code| {
        let code = code as u8;
        standard_text(map, code)
            .or_else(|| map.get(code))
            .map(String::from)
    }))
}

/// The character the standard encoding's `code` reads as, where
/// pdf_encoding's table gives the glyph's other Adobe Glyph List code
/// point: `space` and `hyphen` as the no-break and soft forms, and
/// `periodcentered` and `fraction` as the bullet and division operators. A
/// viewer reads each as the first.
fn standard_text(map: &'static ForwardMap, code: u8) -> Option<char> {
    if !std::ptr::eq(map, &pdf_encoding::STANDARD) {
        return None;
    }
    match code {
        0x20 => Some(' '),
        0x2D => Some('-'),
        0xA4 => Some('\u{2044}'),
        0xB4 => Some('\u{00B7}'),
        _ => None,
    }
}

/// The text the glyph name `name` stands for: the Adobe Glyph List's, or
/// the code points a `uniXXXX` or `uXXXX[XX]` name spells.
fn glyph_text(name: &[u8]) -> Option<String> {
    let name = std::str::from_utf8(name).ok()?;
    // A suffix (`a.alt`) names a variant of the same glyph.
    let base = name.split('.').next().unwrap_or(name);
    if let Some(text) = glyphname_to_unicode(base) {
        return Some(text.to_owned());
    }
    let scalar = |hex: &str| char::from_u32(u32::from_str_radix(hex, 16).ok()?);
    if let Some(hex) = base.strip_prefix("uni")
        && !hex.is_empty()
        && hex.len().is_multiple_of(4)
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return hex
            .as_bytes()
            .chunks(4)
            .map(|digits| scalar(std::str::from_utf8(digits).ok()?))
            .collect();
    }
    if let Some(hex) = base.strip_prefix('u')
        && (4..=6).contains(&hex.len())
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return scalar(hex).map(String::from);
    }
    None
}

/// The encoding CMap of the composite `font`: `Identity-H`/`-V`, a
/// predefined CMap by its name, or a CMap the font embeds (following any
/// `usecmap` to a predefined base); `None` when it names no CMap known or
/// cannot be read.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if an
/// embedded CMap decodes past the content limit.
fn encoding_cmap(font: &Dict<'_>) -> Result<Option<(CMap, Codespace)>> {
    let read = |data: &[u8]| Some((CMap::parse(data, load_embedded)?, Codespace::read(data)?));
    if let Some(name) = font.name(b"Encoding") {
        return Ok(match &*name {
            b"Identity-H" => Some((CMap::identity_h(), Codespace::identity())),
            b"Identity-V" => Some((CMap::identity_v(), Codespace::identity())),
            name => load_embedded(CMapName::from_bytes(name)).and_then(read),
        });
    }
    let Some(stream) = font.get::<Stream<'_>>(b"Encoding") else {
        return Ok(None);
    };
    // A base CMap the stream's dictionary names (`/UseCMap`, 9.7.5.3) is
    // followed as the `usecmap` the stream itself could have written, when
    // it is predefined. One given as a stream is not: hayro-cmap resolves a
    // base by name alone, so the font's codes are not read without it.
    let base = match stream.dict().get::<Object<'_>>(b"UseCMap") {
        None | Some(Object::Null(_)) => Vec::new(),
        Some(Object::Name(name)) => [b"/", &*name, b" usecmap\n"].concat(),
        Some(_) => return Ok(None),
    };
    Ok(match stream.plain() {
        Ok(data) => read(&[base, data].concat()),
        Err(e) if e.kind() == ErrorKind::ResourceLimit => return Err(e),
        Err(_) => None,
    })
}

/// The CMap giving the text of each CID in the composite `font`'s character
/// collection: the one its descendant CIDFont's `/CIDSystemInfo` names, which
/// defines its CIDs, else the one its `encoding` names; `None` for a
/// collection with no such CMap (`Adobe-Identity`, or one of its own).
fn collection_text(font: &Dict<'_>, encoding: &CMap) -> Option<CMap> {
    let named = font
        .get::<Array<'_>>(b"DescendantFonts")
        .and_then(|fonts| fonts.iter::<Dict<'_>>().next())
        .and_then(|cid_font| cid_font.get::<Dict<'_>>(b"CIDSystemInfo"))
        .and_then(|info| {
            let text = |key: &[u8]| {
                info.get::<hayro_syntax::object::String<'_>>(key)
                    .map(|s| s.to_vec())
            };
            Some(CidFamily::from_registry_ordering(
                &text(b"Registry")?,
                &text(b"Ordering")?,
            ))
        });
    let family = named.or_else(|| {
        encoding
            .metadata()
            .character_collection
            .as_ref()
            .map(|collection| collection.family.clone())
    })?;
    CMap::parse(load_embedded(family.ucs2_cmap()?)?, load_embedded)
}

/// The built-in encoding of the simple `font`, which names none: the
/// standard encoding, the standard Symbol and ZapfDingbats fonts' own, or
/// none known for a font whose encoding is its font program's own
/// (9.6.5.1): one flagged symbolic, or a Type1 font it embeds.
fn built_in(font: &Dict<'_>) -> Option<&'static ForwardMap> {
    let base = font.name(b"BaseFont");
    let base = base
        .as_deref()
        .map(|name| name.split(|&b| b == b'+').next_back().unwrap_or(name));
    let descriptor = font.get::<Dict<'_>>(b"FontDescriptor");
    let embedded = descriptor
        .as_ref()
        .is_some_and(|d| d.contains_key(b"FontFile") || d.contains_key(b"FontFile3"));
    match base {
        Some(b"Symbol") if !embedded => return Some(&pdf_encoding::SYMBOL),
        Some(b"ZapfDingbats") if !embedded => return Some(&pdf_encoding::ZDINGBAT),
        _ => {}
    }
    let type1 = font
        .name(b"Subtype")
        .is_some_and(|subtype| matches!(&*subtype, b"Type1" | b"MMType1"));
    if type1 && embedded {
        return None;
    }
    let flags = descriptor
        .and_then(|descriptor| descriptor.number(b"Flags"))
        .map_or(0, |flags| flags as i64);
    // Symbolic (bit 3) and not Nonsymbolic (bit 6), Table 121.
    let symbolic = flags & 4 != 0 && flags & 32 == 0;
    (!symbolic).then_some(&pdf_encoding::STANDARD)
}

/// Whether the predefined CMap `name` maps UCS-2 or UTF-16 codes (9.7.5.2).
fn is_unicode_cmap(name: &[u8]) -> bool {
    name.starts_with(b"Uni") && (contains(name, b"-UCS2-") || contains(name, b"-UTF16-"))
}

/// Whether `name` holds `part`.
fn contains(name: &[u8], part: &[u8]) -> bool {
    name.windows(part.len()).any(|window| window == part)
}

/// The most codes a `/ToUnicode` CMap may map, its single codes and the
/// codes its ranges span together. A CMap the rebuild writes back lists
/// each code it maps, so a range of four-byte codes could name billions;
/// and the count is the same however the codes are written, so a CMap the
/// rebuild writes back, its single codes gathered into ranges, still fits.
const MAX_CODES: u64 = 1 << 18;

/// `stream` read as a `/ToUnicode` CMap, as detection decodes text with it;
/// `None` when it can't be decoded or read.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if it
/// decodes past the content limit, or maps more than [`MAX_CODES`] codes.
pub(crate) fn to_unicode(stream: &Stream<'_>) -> Result<Option<CMap>> {
    let content = match stream.plain() {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::ResourceLimit => return Err(e),
        Err(_) => return Ok(None),
    };
    to_unicode_bytes(&content)
}

/// The CMap `content` read as a `/ToUnicode` CMap, as [`to_unicode`].
///
/// # Errors
///
/// As [`to_unicode`].
pub(crate) fn to_unicode_bytes(content: &[u8]) -> Result<Option<CMap>> {
    let codes = mapped(content)
        .iter()
        .fold(0u64, |count, entry| count.saturating_add(entry.count()));
    if codes > MAX_CODES {
        return Err(Error::resource_limit(format!(
            "a /ToUnicode CMap maps more than {MAX_CODES} codes"
        )));
    }
    Ok(CMap::parse(content, load_embedded))
}

/// The UTF-16 units `cmap` maps `code` to.
pub(crate) fn mapped_units(cmap: &CMap, code: u32) -> Option<Vec<u16>> {
    Some(match cmap.lookup_bf_string(code)? {
        BfString::Char(c) => c.encode_utf16(&mut [0; 2]).to_vec(),
        BfString::String(text) => text.encode_utf16().collect(),
    })
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
pub(crate) fn decode_glyphs(font: &FontText, bytes: &[u8]) -> Result<Vec<Glyph>> {
    match font {
        // Codes split by the encoding: each the shortest run of bytes its
        // ranges map, read through `/ToUnicode`, else as UTF-16 under a
        // Unicode encoding.
        FontText::Composite(composite) => {
            let Composite {
                encoding,
                codespace,
                to_unicode,
                unicode,
                collection,
            } = &**composite;
            let number = |code: &[u8]| code.iter().fold(0, |code, &b| code << 8 | u32::from(b));
            // A code is as long as the codespace range it falls in, mapped to
            // a CID or not; a code no range of the font's own holds is as
            // long as the mapping a predefined CMap gives it.
            let code_len = |rest: &[u8]| {
                codespace.code_len(rest).or_else(|| {
                    (1..=4u8).find_map(|len| {
                        let code = rest.get(..usize::from(len))?;
                        encoding
                            .lookup_cid_code(number(code), len)
                            .map(|_| usize::from(len))
                    })
                })
            };
            let mut glyphs = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                let Some(len) = code_len(&bytes[i..]) else {
                    return Err(Error::redaction(
                        "text drawn with a code outside its font's codespace; its \
                         glyph cannot be located for redaction",
                    ));
                };
                let code_bytes = &bytes[i..i + len];
                let code = number(code_bytes);
                // A code the encoding maps to no CID draws the missing glyph.
                let cid = encoding.lookup_cid_code(code, len as u8).unwrap_or(0);
                let mapped = to_unicode
                    .as_ref()
                    .and_then(|cmap| mapped_units(cmap, code));
                let cid_text = || mapped_units(collection.as_ref()?, cid);
                let units = match mapped {
                    Some(units) => units,
                    None if *unicode && len.is_multiple_of(2) => code_bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&pair| u16::from_be_bytes(pair))
                        .collect(),
                    None if let Some(units) = cid_text() => units,
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
                    width_key: cid,
                    text: mapped_text(&units)?,
                });
                i += len;
            }
            Ok(glyphs)
        }
        // Simple font: one byte per glyph, read through `/ToUnicode` where it
        // maps the byte, else the encoding; a byte neither decodes is likewise
        // an unmatchable glyph.
        FontText::Simple(simple) => {
            let Simple {
                encoding,
                to_unicode,
            } = &**simple;
            bytes
                .iter()
                .enumerate()
                .map(|(i, &b)| {
                    let mapped = to_unicode
                        .as_ref()
                        .and_then(|cmap| mapped_units(cmap, u32::from(b)));
                    let text = match mapped {
                        Some(units) => mapped_text(&units)?,
                        None => encoding
                            .as_ref()
                            .and_then(|encoding| encoding[usize::from(b)].as_deref())
                            .map(readable)
                            .ok_or_else(|| {
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
                        width_key: u32::from(b),
                        text,
                    })
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{held, objects_of, read_id};
    use crate::pdf::object::{Dictionary, Stream as Written, Value, dict};
    use crate::pdf::writer::Document;

    /// The text decoding of `font`, written into `doc` with the streams it
    /// holds as objects of their own.
    fn text_of(mut doc: Document, font: Dictionary) -> Result<Option<FontText>> {
        let font = objects_of(&mut doc, Value::Dict(font));
        let font = doc.add(font);
        let pdf = held(doc, &[font]);
        let dict = pdf.object(read_id(font)).unwrap().into_dict().unwrap();
        FontText::of(&dict)
    }

    /// The CMap `stream` read as a `/ToUnicode` CMap.
    fn unicode_of(stream: Written) -> Result<Option<CMap>> {
        let mut doc = Document::new((1, 7));
        let id = doc.add(stream);
        let pdf = held(doc, &[id]);
        to_unicode(&pdf.object(read_id(id)).unwrap().into_stream().unwrap())
    }

    /// A one-byte-and-two-byte `/ToUnicode` CMap mapping `<41>` to `J`,
    /// `<8140>` to `a` (as a Shift-JIS font's would), and `<5C71>` to `x`.
    const TO_UNICODE: &[u8] = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
        /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
        /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
        2 begincodespacerange\n<00> <80>\n<8140> <9FFC>\nendcodespacerange\n\
        3 beginbfchar\n<41> <004A>\n<8140> <0061>\n<5C71> <0078>\nendbfchar\n\
        endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";

    /// The glyphs `bytes` draw in a composite font of `encoding`, with the
    /// `/ToUnicode` above when `mapped`.
    fn composite(encoding: Value, mapped: bool, bytes: &[u8]) -> Option<Result<Vec<String>>> {
        let mut doc = Document::new((1, 7));
        let mut font = dict! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X", "Encoding" => encoding,
        };
        if mapped {
            font.set(
                "ToUnicode",
                doc.add(Written::new(dict! {}, TO_UNICODE.to_vec())),
            );
        }
        let text = text_of(doc.clone(), font.clone()).unwrap()?;
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
        let embedded = Value::from(Written::new(
            dict! { "Type" => "CMap" },
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              2 begincodespacerange\n<00> <80>\n<8140> <9FFC>\nendcodespacerange\n\
              2 begincidrange\n<41> <41> 34\n<8140> <8140> 633\nendcidrange\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
                .to_vec(),
        ));
        for encoding in [Value::Name(b"90ms-RKSJ-H".to_vec()), embedded] {
            let glyphs = composite(encoding, true, b"\x41\x81\x40").unwrap().unwrap();
            assert_eq!(glyphs, ["J", "a"]);
        }
    }

    /// A code in the encoding's codespace that maps to no CID is still a
    /// code: its `/ToUnicode` reads it, and it draws the missing glyph (CID
    /// 0), whose width it takes; a mapped code takes its CID's.
    #[test]
    fn a_code_in_the_codespace_without_a_cid_still_reads() {
        let mut doc = Document::new((1, 7));
        let encoding = doc.add(Written::new(
            dict! { "Type" => "CMap" },
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              2 begincodespacerange\n<00> <80>\n<8140> <9FFC>\nendcodespacerange\n\
              1 begincidrange\n<41> <41> 34\nendcidrange\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
                .to_vec(),
        ));
        let to_unicode = doc.add(Written::new(dict! {}, TO_UNICODE.to_vec()));
        let font = dict! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X",
            "Encoding" => encoding, "ToUnicode" => to_unicode,
        };
        let text = text_of(doc.clone(), font.clone()).unwrap().unwrap();
        let glyphs = decode_glyphs(&text, b"\x41\x81\x40").unwrap();
        let read: Vec<(&str, u32, u32)> = glyphs
            .iter()
            .map(|glyph| (glyph.text.as_str(), glyph.code, glyph.width_key))
            .collect();
        assert_eq!(read, [("J", 0x41, 34), ("a", 0x8140, 0)]);
    }

    /// An encoding whose base CMap is a stream (`/UseCMap`) is not followed,
    /// so the font's codes are not read; one naming a predefined base reads
    /// through it.
    #[test]
    fn an_encoding_on_a_stream_base_is_refused() {
        let named = Value::from(Written::new(
            dict! { "Type" => "CMap", "UseCMap" => "90ms-RKSJ-H" },
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
                .to_vec(),
        ));
        assert_eq!(
            composite(named, true, b"\x41\x81\x40").unwrap().unwrap(),
            ["J", "a"]
        );
        let base = Written::new(
            dict! { "Type" => "CMap" },
            b"begincmap\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\nendcmap".to_vec(),
        );
        let child = Value::from(Written::new(
            dict! { "Type" => "CMap", "UseCMap" => Value::from(base) },
            b"begincmap\nendcmap".to_vec(),
        ));
        assert!(composite(child, true, b"\x41").is_none());
    }

    /// A code the `/ToUnicode` does not map reads as its CID's text in the
    /// encoding's collection, not as nothing: under `90ms-RKSJ-H`, `0x42` is
    /// Adobe-Japan1's `B`.
    #[test]
    fn a_code_its_to_unicode_lacks_reads_through_the_collection() {
        let name = Value::Name(b"90ms-RKSJ-H".to_vec());
        assert_eq!(
            composite(name, true, b"\x41\x42").unwrap().unwrap(),
            ["J", "B"]
        );
    }

    /// A composite font under a predefined Unicode CMap and no `/ToUnicode`
    /// reads its codes as UTF-16, a surrogate pair (here 𠮟, in the Japanese
    /// collection) as one glyph.
    #[test]
    fn a_unicode_cmap_reads_as_utf16() {
        let name = Value::Name(b"UniJIS-UTF16-H".to_vec());
        let glyphs = composite(name, false, b"\x00J\xD8\x42\xDF\x9F")
            .unwrap()
            .unwrap();
        assert_eq!(glyphs, ["J", "\u{20B9F}"]);
    }

    /// How many codes the CMap `content` maps.
    fn mapped_codes(content: &[u8]) -> u64 {
        mapped(content).iter().map(|entry| entry.count()).sum()
    }

    /// Codes are counted as their entries span them: a CMap spanning too
    /// many codes is refused, one in a comment is no range, and single
    /// codes count as ranges do, so the count is the same however the
    /// codes are written.
    #[test]
    fn ranges_are_counted_before_they_are_read() {
        let wide = b"1 beginbfrange\n<000000> <04FFFF> <0041>\nendbfrange\n";
        assert!(mapped_codes(wide) > MAX_CODES);
        let commented =
            b"1 beginbfrange\n% <000000> <04FFFF> <0041>\n<01> <02> <0041>\nendbfrange\n";
        assert_eq!(mapped_codes(commented), 2);
        let array = b"1 beginbfrange\n<01> <02> [<0041> <0042>]\n<10> <10> <0043>\nendbfrange\n";
        assert_eq!(mapped_codes(array), 3);
        let chars = b"2 beginbfchar\n<01> <0041>\n<02> <0042>\nendbfchar\n\
            1 beginbfrange\n<03> <04> <0043>\nendbfrange\n";
        assert_eq!(mapped_codes(chars), 4);
    }

    /// A `%` inside a string, or a `>` inside a dictionary, hides no range
    /// from the count.
    #[test]
    fn strings_and_dictionaries_hide_no_range() {
        let wide = b"/CIDSystemInfo << /Registry (%) /Ordering (a>b) >> def \
            1 beginbfrange <000000> <04FFFF> <0041> endbfrange";
        assert!(mapped_codes(wide) > MAX_CODES);
    }

    /// A CMap whose filter is not one is not read at all, so no bytes that
    /// went uncounted are parsed.
    #[test]
    fn a_cmap_under_a_filter_that_is_none_is_unreadable() {
        let stream = Written::new(dict! { "Filter" => 5 }, TO_UNICODE.to_vec());
        assert!(unicode_of(stream).unwrap().is_none());
    }

    /// A composite font with no `/ToUnicode`, no Unicode encoding and no
    /// character collection with text has none to read: under `Identity-H`
    /// alone its codes split, but read as nothing. One whose encoding names
    /// no CMap can't even be split.
    #[test]
    fn a_composite_font_with_nothing_to_read_is_refused() {
        let refused = composite(Value::Name(b"Identity-H".to_vec()), false, b"\x00\x41");
        assert!(refused.unwrap().is_err());
        assert!(composite(Value::Name(b"No-Such-CMap".to_vec()), true, b"\x41").is_none());
    }

    /// The glyphs `bytes` draw in a composite font of `encoding` whose
    /// descendant CIDFont names the collection `registry`-`ordering`, with no
    /// `/ToUnicode`.
    fn collected(encoding: &str, ordering: &str, bytes: &[u8]) -> Result<Vec<String>> {
        let mut doc = Document::new((1, 7));
        let descendant = doc.add(dict! {
            "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "X",
            "CIDSystemInfo" => dict! {
                "Registry" => Value::string("Adobe"),
                "Ordering" => Value::string(ordering),
                "Supplement" => 6,
            },
        });
        let font = dict! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X",
            "Encoding" => encoding, "DescendantFonts" => vec![descendant.into()],
        };
        let text = text_of(doc.clone(), font.clone()).unwrap().unwrap();
        decode_glyphs(&text, bytes).map(|glyphs| glyphs.into_iter().map(|g| g.text).collect())
    }

    /// With no `/ToUnicode`, a code reads as its CID's text in the font's
    /// character collection: Adobe-Japan1 CID 34 is `A`, reached directly
    /// under `Identity-H` and through Shift-JIS under `90ms-RKSJ-H`.
    #[test]
    fn a_code_reads_as_its_cids_text_in_the_collection() {
        assert_eq!(
            collected("Identity-H", "Japan1", b"\x00\x22").unwrap(),
            ["A"]
        );
        assert_eq!(collected("90ms-RKSJ-H", "Japan1", b"\x41").unwrap(), ["A"]);
        let err = collected("Identity-H", "Identity", b"\x00\x22").unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::Redaction);
    }

    /// Codes split by the encoding's ranges, whatever the `/ToUnicode` maps:
    /// under Shift-JIS a lead byte starts a two-byte code even where the map
    /// has a one-byte code of that value.
    #[test]
    fn codes_split_by_the_encoding_not_the_map() {
        let name = Value::Name(b"90ms-RKSJ-H".to_vec());
        let glyphs = composite(name, true, b"\x81\x40\x41").unwrap().unwrap();
        assert_eq!(glyphs, ["a", "J"]);
    }

    /// Under a Unicode encoding, a code the `/ToUnicode` does not map reads
    /// as the UTF-16 it is.
    #[test]
    fn a_unicode_encoding_reads_a_code_its_to_unicode_lacks() {
        let name = Value::Name(b"UniJIS-UCS2-H".to_vec());
        let glyphs = composite(name, true, b"\x5C\x71\x00\x62").unwrap().unwrap();
        assert_eq!(glyphs, ["x", "b"]);
    }

    /// Codes split by the encoding, not by what the `/ToUnicode` maps: under
    /// `Identity-H` a one-byte mapping never shortens a two-byte code.
    #[test]
    fn identity_codes_are_two_bytes_whatever_the_map_holds() {
        let name = Value::Name(b"Identity-H".to_vec());
        let err = composite(name, true, b"\x41\x81").unwrap().unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::Redaction);
    }

    /// A simple font with no `/Encoding` reads a byte its `/ToUnicode` lacks
    /// through the standard encoding, still one byte a code.
    #[test]
    fn a_simple_font_without_an_encoding_falls_back_per_byte() {
        let mut doc = Document::new((1, 7));
        let map = doc.add(Written::new(dict! {}, TO_UNICODE.to_vec()));
        let font = dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => map,
        };
        let text = text_of(doc.clone(), font.clone()).unwrap().unwrap();
        let glyphs: Vec<String> = decode_glyphs(&text, b"\x41\x42")
            .unwrap()
            .into_iter()
            .map(|glyph| glyph.text)
            .collect();
        assert_eq!(glyphs, ["J", "B"]);
    }

    /// A symbolic font, or an embedded Type1 font, with no `/Encoding` has a
    /// built-in encoding of its own, so a byte its `/ToUnicode` lacks is
    /// refused, not read as the standard encoding's letter; and a simple font
    /// naming a composite encoding is no font whose codes can be read.
    #[test]
    fn a_symbolic_font_refuses_a_byte_its_map_lacks() {
        let mut doc = Document::new((1, 7));
        let map = doc.add(Written::new(dict! {}, TO_UNICODE.to_vec()));
        let descriptor = doc.add(dict! { "Type" => "FontDescriptor", "Flags" => 4 });
        let program = doc.add(Written::new(dict! {}, Vec::new()));
        let embedded = doc.add(dict! {
            "Type" => "FontDescriptor", "Flags" => 32, "FontFile" => program,
        });
        // The standard Symbol font's own encoding is the standard Symbol
        // table, which is read.
        let symbol = dict! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Symbol", "ToUnicode" => map };
        let text = text_of(doc.clone(), symbol).unwrap().unwrap();
        assert_eq!(decode_glyphs(&text, b"\x42").unwrap()[0].text, "\u{0392}");
        for font in [
            dict! {
                "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "ABCDEF+Glyphs",
                "ToUnicode" => map, "FontDescriptor" => descriptor,
            },
            dict! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ABCDEF+Embedded",
                "ToUnicode" => map, "FontDescriptor" => embedded,
            },
        ] {
            let text = text_of(doc.clone(), font.clone()).unwrap().unwrap();
            assert_eq!(decode_glyphs(&text, b"\x41").unwrap()[0].text, "J");
            let refused = decode_glyphs(&text, b"\x42");
            assert!(refused.is_err_and(|e| e.kind() == ErrorKind::Redaction));
        }
        let composite = dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X",
            "Encoding" => "Identity-H", "ToUnicode" => map,
        };
        assert!(text_of(doc.clone(), composite.clone()).unwrap().is_none());
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
