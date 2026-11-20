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

use std::{array, iter};

use elide_core::{Error, ErrorKind, Result};
use hayro_cmap::{BfString, CMap, CMapName, CidFamily, WritingMode, load_embedded};
use lopdf::{Dictionary, Document, Encoding, Object, Stream, dictionary};
use unicode_normalization::UnicodeNormalization;

use super::cmap::{Codespace, mapped_codes};
use super::encoding::{self, BaseEncoding, NONSYMBOLIC, SYMBOLIC};
use super::program::built_in_names;
use super::{Standard, descendant, subset_free};
use crate::pdf::reader::{DictionaryExt, DocumentExt, StreamExt};

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
/// `/ToUnicode` where that maps the code, else as `codes` gives it.
pub(crate) struct Simple {
    /// Each code's text through the font's encoding, `None` for a code whose
    /// text can't be known.
    codes: Box<[Option<String>; 256]>,
    to_unicode: Option<Encoding<'static>>,
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
    to_unicode: Option<Encoding<'static>>,
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
    pub(crate) fn of(doc: &Document, font: &Dictionary) -> Result<Option<Self>> {
        let to_unicode = match font.get(b"ToUnicode").ok().and_then(|c| doc.resolve(c)) {
            Some(Object::Stream(stream)) => to_unicode(doc, stream)?,
            _ => None,
        };
        if font.is_subtype(doc, b"Type0") {
            let unicode = font.name(doc, b"Encoding").is_some_and(is_unicode_cmap);
            let Some((encoding, codespace)) = encoding_cmap(doc, font)? else {
                return Ok(None);
            };
            let collection = collection_text(doc, font, &encoding);
            return Ok(Some(Self::Composite(Box::new(Composite {
                encoding,
                codespace,
                to_unicode,
                unicode,
                collection,
            }))));
        }
        // A composite encoding is no simple font's.
        let base = match font.get(b"Encoding").ok().and_then(|e| doc.resolve(e)) {
            Some(Object::Dictionary(dict)) => dict.name(doc, b"BaseEncoding"),
            Some(Object::Name(name)) => Some(name.as_slice()),
            _ => None,
        };
        if matches!(base, Some(b"Identity-H" | b"Identity-V")) {
            return Ok(None);
        }
        let codes = match Standard::of(doc, font) {
            Some(standard) => Box::new(
                standard
                    .text(doc, font)
                    .map(|text| Some(text.unwrap_or_default())),
            ),
            None => encoded_text(doc, font),
        };
        Ok(Some(Self::Simple(Box::new(Simple { codes, to_unicode }))))
    }
}

impl FontText {
    /// Whether the font writes vertically: a composite font whose encoding
    /// CMap's writing mode is vertical (9.7.4.3), whose glyphs advance down
    /// the page by metrics of their own.
    pub(crate) fn is_vertical(&self) -> bool {
        matches!(
            self,
            FontText::Composite(composite)
                if composite.encoding.metadata().writing_mode == Some(WritingMode::Vertical)
        )
    }
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
fn encoding_cmap(doc: &Document, font: &Dictionary) -> Result<Option<(CMap, Codespace)>> {
    let read = |data: &[u8]| Some((CMap::parse(data, load_embedded)?, Codespace::read(data)?));
    Ok(
        match font.get(b"Encoding").ok().and_then(|e| doc.resolve(e)) {
            Some(Object::Name(name)) => match name.as_slice() {
                b"Identity-H" => Some((CMap::identity_h(), Codespace::identity())),
                b"Identity-V" => Some((CMap::identity_v(), Codespace::identity())),
                name => load_embedded(CMapName::from_bytes(name)).and_then(read),
            },
            // A base CMap the stream's dictionary names (`/UseCMap`, 9.7.5.3)
            // is followed as the `usecmap` the stream itself could have
            // written, when it is predefined. One given as a stream is not:
            // hayro-cmap resolves a base by name alone, so the font's codes
            // are not read without it.
            Some(Object::Stream(stream)) => {
                let base = match stream
                    .dict
                    .get(b"UseCMap")
                    .ok()
                    .and_then(|b| doc.resolve(b))
                {
                    None | Some(Object::Null) => Vec::new(),
                    Some(Object::Name(name)) => [b"/", name.as_slice(), b" usecmap\n"].concat(),
                    Some(_) => return Ok(None),
                };
                match stream.decoded() {
                    Ok(data) => read(&[base, data].concat()),
                    Err(e) if e.kind() == ErrorKind::ResourceLimit => return Err(e),
                    Err(_) => None,
                }
            }
            _ => None,
        },
    )
}

/// The CMap giving the text of each CID in the composite `font`'s character
/// collection: the one its descendant CIDFont's `/CIDSystemInfo` names, which
/// defines its CIDs, else the one its `encoding` names; `None` for a
/// collection with no such CMap (`Adobe-Identity`, or one of its own).
fn collection_text(doc: &Document, font: &Dictionary, encoding: &CMap) -> Option<CMap> {
    let named = descendant(doc, font)
        .and_then(|cid_font| doc.resolve_dict(cid_font.get(b"CIDSystemInfo").ok()?))
        .and_then(|info| {
            let text = |key: &[u8]| doc.resolve(info.get(key).ok()?)?.as_str().ok();
            Some(CidFamily::from_registry_ordering(
                text(b"Registry")?,
                text(b"Ordering")?,
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

/// The text each code of the simple `font`, which names no standard face,
/// reads as through its encoding (9.6.5): the glyph name its `/Encoding`
/// gives the code, through the Adobe Glyph List.
///
/// The base is the encoding the font names; else, for a font whose encoding
/// is its program's own (the Symbol and ZapfDingbats faces it embeds, a
/// Type1 font it embeds, or one flagged symbolic only), the built-in
/// encoding of the program it embeds, when that is read; else the standard
/// encoding. A code the base gives no glyph, or that names `.notdef`, draws
/// nothing and reads as no text. A code whose text can't be known is `None`:
/// one only a built-in encoding that isn't read would name, or one whose
/// glyph name spells no text.
fn encoded_text(doc: &Document, font: &Dictionary) -> Box<[Option<String>; 256]> {
    let encoding = encoding::Encoding::of(doc, font);
    let base = encoding.base().map(BaseEncoding::names).or_else(|| {
        let own = matches!(
            font.name(doc, b"BaseFont").map(subset_free),
            Some(b"Symbol" | b"ZapfDingbats")
        ) || embeds_type1(doc, font)
            || {
                let flags = encoding::flags(doc, font);
                flags & SYMBOLIC != 0 && flags & NONSYMBOLIC == 0
            };
        (!own).then(|| BaseEncoding::Standard.names())
    });
    let built_in = base.is_none().then(|| built_in_names(doc, font)).flatten();
    let known = base.is_some() || built_in.is_some();
    let mut names = base.unwrap_or([None; 256]);
    encoding.apply(doc, &mut names);
    let dingbats = font.name(doc, b"BaseFont").map(subset_free) == Some(b"ZapfDingbats");
    let text = |name: &str| match name {
        ".notdef" => Some(String::new()),
        name => encoding::text(name, dingbats),
    };
    Box::new(array::from_fn(|code| {
        match (
            names[code],
            built_in.as_ref().and_then(|names| names[code].as_deref()),
        ) {
            (Some(name), _) | (None, Some(name)) => text(name),
            (None, None) => known.then(String::new),
        }
    }))
}

/// Whether the Type1 `font` embeds its program.
fn embeds_type1(doc: &Document, font: &Dictionary) -> bool {
    matches!(font.name(doc, b"Subtype"), Some(b"Type1" | b"MMType1"))
        && font
            .get(b"FontDescriptor")
            .ok()
            .and_then(|descriptor| doc.resolve_dict(descriptor))
            .is_some_and(|d| d.get(b"FontFile").is_ok() || d.get(b"FontFile3").is_ok())
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
/// codes its ranges span together. lopdf expands every code of every range
/// as it parses, so a range of four-byte codes could name billions; and the
/// count is the same however the codes are written, so a CMap the rebuild
/// writes back, its single codes gathered into ranges, still fits.
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
/// decodes past the content limit, or maps more than [`MAX_CODES`] codes,
/// counted before lopdf expands them.
pub(crate) fn to_unicode(doc: &Document, stream: &Stream) -> Result<Option<Encoding<'static>>> {
    let content = match stream.decoded() {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::ResourceLimit => return Err(e),
        Err(_) => return Ok(None),
    };
    if mapped_codes(&content) > MAX_CODES {
        return Err(Error::resource_limit(format!(
            "a /ToUnicode CMap maps more than {MAX_CODES} codes"
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
                let mapped = match to_unicode {
                    Some(Encoding::UnicodeMapEncoding(cmap)) => cmap.get(code, len as u8),
                    _ => None,
                };
                let cid_text = || match collection.as_ref()?.lookup_bf_string(cid)? {
                    BfString::Char(c) => Some(c.encode_utf16(&mut [0; 2]).to_vec()),
                    BfString::String(text) => Some(text.encode_utf16().collect()),
                };
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
            let Simple { codes, to_unicode } = &**simple;
            bytes
                .iter()
                .enumerate()
                .map(|(i, &b)| {
                    let mapped = match to_unicode {
                        Some(Encoding::UnicodeMapEncoding(cmap)) => cmap.get(u32::from(b), 1),
                        _ => None,
                    };
                    let text = match mapped {
                        Some(units) => mapped_text(&units)?,
                        None => {
                            codes[usize::from(b)]
                                .as_deref()
                                .map(readable)
                                .ok_or_else(|| {
                                    Error::redaction(
                                        "text drawn with a byte that does not decode under its \
                                 font; its glyph cannot be located for redaction",
                                    )
                                })?
                        }
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
    use crate::fixtures::{FontFile, Type1, embedding};

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
        let embedded = Object::Stream(Stream::new(
            dictionary! { "Type" => "CMap" },
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              2 begincodespacerange\n<00> <80>\n<8140> <9FFC>\nendcodespacerange\n\
              2 begincidrange\n<41> <41> 34\n<8140> <8140> 633\nendcidrange\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
                .to_vec(),
        ));
        for encoding in [Object::Name(b"90ms-RKSJ-H".to_vec()), embedded] {
            let glyphs = composite(encoding, true, b"\x41\x81\x40").unwrap().unwrap();
            assert_eq!(glyphs, ["J", "a"]);
        }
    }

    /// A code in the encoding's codespace that maps to no CID is still a
    /// code: its `/ToUnicode` reads it, and it draws the missing glyph (CID
    /// 0), whose width it takes; a mapped code takes its CID's.
    #[test]
    fn a_code_in_the_codespace_without_a_cid_still_reads() {
        let mut doc = Document::with_version("1.7");
        let encoding = doc.add_object(Stream::new(
            dictionary! { "Type" => "CMap" },
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              2 begincodespacerange\n<00> <80>\n<8140> <9FFC>\nendcodespacerange\n\
              1 begincidrange\n<41> <41> 34\nendcidrange\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
                .to_vec(),
        ));
        let to_unicode = doc.add_object(Stream::new(dictionary! {}, TO_UNICODE.to_vec()));
        let font = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X",
            "Encoding" => encoding, "ToUnicode" => to_unicode,
        };
        let text = FontText::of(&doc, &font).unwrap().unwrap();
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
        let named = Object::Stream(Stream::new(
            dictionary! { "Type" => "CMap", "UseCMap" => "90ms-RKSJ-H" },
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
              endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
                .to_vec(),
        ));
        assert_eq!(
            composite(named, true, b"\x41\x81\x40").unwrap().unwrap(),
            ["J", "a"]
        );
        let base = Stream::new(
            dictionary! { "Type" => "CMap" },
            b"begincmap\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\nendcmap".to_vec(),
        );
        let child = Object::Stream(Stream::new(
            dictionary! { "Type" => "CMap", "UseCMap" => Object::Stream(base) },
            b"begincmap\nendcmap".to_vec(),
        ));
        assert!(composite(child, true, b"\x41").is_none());
    }

    /// A code the `/ToUnicode` does not map reads as its CID's text in the
    /// encoding's collection, not as nothing: under `90ms-RKSJ-H`, `0x42` is
    /// Adobe-Japan1's `B`.
    #[test]
    fn a_code_its_to_unicode_lacks_reads_through_the_collection() {
        let name = Object::Name(b"90ms-RKSJ-H".to_vec());
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
        let name = Object::Name(b"UniJIS-UTF16-H".to_vec());
        let glyphs = composite(name, false, b"\x00J\xD8\x42\xDF\x9F")
            .unwrap()
            .unwrap();
        assert_eq!(glyphs, ["J", "\u{20B9F}"]);
    }

    /// Codes are counted before lopdf expands them: a CMap spanning too
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

    /// A CMap whose filter is not one is not read at all, so lopdf never
    /// parses bytes that went uncounted.
    #[test]
    fn a_cmap_under_a_filter_that_is_none_is_unreadable() {
        let stream = Stream::new(dictionary! { "Filter" => 5 }, TO_UNICODE.to_vec());
        assert!(to_unicode(&Document::new(), &stream).unwrap().is_none());
    }

    /// A composite font with no `/ToUnicode`, no Unicode encoding and no
    /// character collection with text has none to read: under `Identity-H`
    /// alone its codes split, but read as nothing. One whose encoding names
    /// no CMap can't even be split.
    #[test]
    fn a_composite_font_with_nothing_to_read_is_refused() {
        let refused = composite(Object::Name(b"Identity-H".to_vec()), false, b"\x00\x41");
        assert!(refused.unwrap().is_err());
        assert!(composite(Object::Name(b"No-Such-CMap".to_vec()), true, b"\x41").is_none());
    }

    /// The glyphs `bytes` draw in a composite font of `encoding` whose
    /// descendant CIDFont names the collection `registry`-`ordering`, with no
    /// `/ToUnicode`.
    fn collected(encoding: &str, ordering: &str, bytes: &[u8]) -> Result<Vec<String>> {
        let mut doc = Document::with_version("1.7");
        let descendant = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "X",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal(ordering),
                "Supplement" => 6,
            },
        });
        let font = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X",
            "Encoding" => encoding, "DescendantFonts" => vec![descendant.into()],
        };
        let text = FontText::of(&doc, &font).unwrap().unwrap();
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
        let name = Object::Name(b"90ms-RKSJ-H".to_vec());
        let glyphs = composite(name, true, b"\x81\x40\x41").unwrap().unwrap();
        assert_eq!(glyphs, ["a", "J"]);
    }

    /// Under a Unicode encoding, a code the `/ToUnicode` does not map reads
    /// as the UTF-16 it is.
    #[test]
    fn a_unicode_encoding_reads_a_code_its_to_unicode_lacks() {
        let name = Object::Name(b"UniJIS-UCS2-H".to_vec());
        let glyphs = composite(name, true, b"\x5C\x71\x00\x62").unwrap().unwrap();
        assert_eq!(glyphs, ["x", "b"]);
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

    /// The text `bytes` draw in a simple font with no `/ToUnicode`, of
    /// `subtype`, embedding a Type1 program when `embedded`, with `encoding`.
    fn encoded(subtype: &str, embedded: bool, encoding: Object, bytes: &[u8]) -> Result<String> {
        let mut doc = Document::with_version("1.7");
        let mut font = dictionary! {
            "Type" => "Font", "Subtype" => subtype, "BaseFont" => "ABCDEF+Custom",
            "Encoding" => encoding,
        };
        if embedded {
            let program = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
            font.set(
                "FontDescriptor",
                dictionary! { "Type" => "FontDescriptor", "Flags" => 32, "FontFile" => program },
            );
        }
        let text = FontText::of(&doc, &font).unwrap().unwrap();
        Ok(decode_glyphs(&text, bytes)?
            .into_iter()
            .map(|glyph| glyph.text)
            .collect())
    }

    /// A simple font's codes read through the glyph names its encoding gives
    /// them, by the Adobe Glyph List: every name it lists, `uniXXXX` names,
    /// and ligatures by their components, whatever other names the
    /// `/Differences` hold.
    #[test]
    fn differences_read_by_glyph_name() {
        let differences = |names: &[&str]| {
            let mut entries = vec![Object::Integer(1)];
            entries.extend(names.iter().map(|&name| Object::from(name)));
            Object::Dictionary(dictionary! { "Differences" => entries })
        };
        let encoding = differences(&["J", "a", "n", "e", "uni2019", "f_i", "a.sc"]);
        assert_eq!(
            encoded("TrueType", false, encoding, b"\x01\x02\x03\x04\x05\x06\x07").unwrap(),
            "Jane\u{2019}fia"
        );
        let winansi = Object::Dictionary(dictionary! {
            "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => vec![0x41.into(), ".notdef".into()],
        });
        assert_eq!(
            encoded("Type1", false, winansi, b"BA\xe9").unwrap(),
            "B\u{e9}"
        );
    }

    /// A code whose glyph name spells no text, or that only an embedded
    /// program's own encoding names, is refused rather than read as nothing.
    #[test]
    fn a_code_whose_text_cant_be_known_is_refused() {
        let custom =
            Object::Dictionary(dictionary! { "Differences" => vec![1.into(), "g7".into()] });
        let refused = encoded("TrueType", false, custom, b"\x01");
        assert!(refused.is_err_and(|e| e.kind() == ErrorKind::Redaction));

        let embedded =
            || Object::Dictionary(dictionary! { "Differences" => vec![1.into(), "J".into()] });
        assert_eq!(encoded("Type1", true, embedded(), b"\x01").unwrap(), "J");
        let refused = encoded("Type1", true, embedded(), b"A");
        assert!(refused.is_err_and(|e| e.kind() == ErrorKind::Redaction));
        assert_eq!(encoded("Type1", false, embedded(), b"A").unwrap(), "A");
    }

    /// An embedded Type 1 program's built-in encoding names the codes the
    /// font's encoding leaves to it, so they read as text; a code it maps no
    /// glyph to draws nothing.
    #[test]
    fn a_built_in_encoding_names_its_codes() {
        let mut doc = Document::with_version("1.7");
        let program = Type1::new(&[(".notdef", 250), ("J", 500), ("a", 500)])
            .encoding(&[(1, "J"), (2, "a")])
            .build();
        let font = embedding(&mut doc, "Type1", FontFile::Type1, program, 4);
        let text = FontText::of(&doc, &font).unwrap().unwrap();
        let glyphs: Vec<String> = decode_glyphs(&text, b"\x01\x02\x03")
            .unwrap()
            .into_iter()
            .map(|glyph| glyph.text)
            .collect();
        assert_eq!(glyphs, ["J", "a", ""]);
    }

    /// A symbolic font, or an embedded Type1 font, with no `/Encoding` has a
    /// built-in encoding of its own, so a byte its `/ToUnicode` lacks is
    /// refused, not read as the standard encoding's letter; the standard
    /// Symbol face's is known, so it reads. A simple font naming a composite
    /// encoding is no font whose codes can be read.
    #[test]
    fn a_symbolic_font_refuses_a_byte_its_map_lacks() {
        let mut doc = Document::with_version("1.7");
        let map = doc.add_object(Stream::new(dictionary! {}, TO_UNICODE.to_vec()));
        let descriptor = doc.add_object(dictionary! { "Type" => "FontDescriptor", "Flags" => 4 });
        let program = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
        let embedded = doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "Flags" => 32, "FontFile" => program,
        });
        let symbol = dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Symbol", "ToUnicode" => map,
        };
        let text = FontText::of(&doc, &symbol).unwrap().unwrap();
        let glyphs: Vec<String> = decode_glyphs(&text, b"\x41\x42")
            .unwrap()
            .into_iter()
            .map(|glyph| glyph.text)
            .collect();
        assert_eq!(glyphs, ["J", "\u{392}"], "Beta, by Symbol's own encoding");
        for font in [
            dictionary! {
                "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "ABCDEF+Glyphs",
                "ToUnicode" => map, "FontDescriptor" => descriptor,
            },
            dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ABCDEF+Embedded",
                "ToUnicode" => map, "FontDescriptor" => embedded,
            },
        ] {
            let text = FontText::of(&doc, &font).unwrap().unwrap();
            assert_eq!(decode_glyphs(&text, b"\x41").unwrap()[0].text, "J");
            let refused = decode_glyphs(&text, b"\x42");
            assert!(refused.is_err_and(|e| e.kind() == ErrorKind::Redaction));
        }
        let composite = dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X",
            "Encoding" => "Identity-H", "ToUnicode" => map,
        };
        assert!(FontText::of(&doc, &composite).unwrap().is_none());
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
