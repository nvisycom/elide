//! The standard 14 fonts (9.6.2.2): the widths and vertical metrics a reader
//! supplies for a simple font that names one and embeds no font program.
//!
//! The metrics are Adobe's Core14 AFM files (`assets/afm`). A font's codes
//! reach its glyphs by name: its encoding names the glyph each code selects
//! (9.6.5), and the AFM gives each glyph name its width. A font may name a
//! face by an alternative name (`Arial`, `TimesNewRoman,Bold`, `CourierNew`),
//! which renderers draw with the face's metrics.
//!
//! Such a font carries no widths of its own, so its glyphs are as wide as the
//! renderer showing it makes them. Where PDFium and pdf.js agree, a glyph has
//! the width they give it, even where they depart from the letter of Annex D;
//! where they disagree, it has none. So a code has no width when its glyph
//! is one the renderers size differently ([`UNSETTLED`]), when they select
//! different glyphs for it, or when it selects a glyph the face lacks.

mod encodings;

use std::array;
use std::collections::HashMap;
use std::str::SplitWhitespace;
use std::sync::OnceLock;

use lopdf::{Dictionary, Document, Object};

use self::encodings::{MAC_ROMAN, WIN_ANSI};
use crate::pdf::reader::{DictionaryExt, DocumentExt};

/// A code's glyph name in a 256-code simple-font encoding.
type GlyphNames<'a> = [Option<&'a str>; 256];

/// The glyphs of the Helvetica and Times faces that PDFium, which draws
/// them with fonts of its own, gives widths other than the AFMs', which
/// pdf.js and PDFBox use: in some style of each family, each of these differs.
const UNSETTLED: &[&str] = &[
    "Lcaron",
    "commaaccent",
    "dcaron",
    "divide",
    "imacron",
    "lcaron",
    "lozenge",
    "macron",
    "mu",
    "partialdiff",
    "periodcentered",
    "plusminus",
    "radical",
    "summation",
    "tcaron",
];

/// The Symbolic and Nonsymbolic font descriptor flags (Table 121).
const SYMBOLIC: i64 = 1 << 2;
const NONSYMBOLIC: i64 = 1 << 5;

/// One of the 14 standard faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Symbol,
    ZapfDingbats,
}

impl Face {
    /// Every face, in declaration order (each one's index into the cache).
    #[cfg(test)]
    const ALL: [Face; 14] = [
        Face::Courier,
        Face::CourierBold,
        Face::CourierOblique,
        Face::CourierBoldOblique,
        Face::Helvetica,
        Face::HelveticaBold,
        Face::HelveticaOblique,
        Face::HelveticaBoldOblique,
        Face::TimesRoman,
        Face::TimesBold,
        Face::TimesItalic,
        Face::TimesBoldItalic,
        Face::Symbol,
        Face::ZapfDingbats,
    ];

    /// The face's AFM file.
    fn afm(self) -> &'static str {
        match self {
            Face::Courier => include_str!("../../../../../assets/afm/Courier.afm"),
            Face::CourierBold => include_str!("../../../../../assets/afm/Courier-Bold.afm"),
            Face::CourierOblique => include_str!("../../../../../assets/afm/Courier-Oblique.afm"),
            Face::CourierBoldOblique => {
                include_str!("../../../../../assets/afm/Courier-BoldOblique.afm")
            }
            Face::Helvetica => include_str!("../../../../../assets/afm/Helvetica.afm"),
            Face::HelveticaBold => include_str!("../../../../../assets/afm/Helvetica-Bold.afm"),
            Face::HelveticaOblique => {
                include_str!("../../../../../assets/afm/Helvetica-Oblique.afm")
            }
            Face::HelveticaBoldOblique => {
                include_str!("../../../../../assets/afm/Helvetica-BoldOblique.afm")
            }
            Face::TimesRoman => include_str!("../../../../../assets/afm/Times-Roman.afm"),
            Face::TimesBold => include_str!("../../../../../assets/afm/Times-Bold.afm"),
            Face::TimesItalic => include_str!("../../../../../assets/afm/Times-Italic.afm"),
            Face::TimesBoldItalic => {
                include_str!("../../../../../assets/afm/Times-BoldItalic.afm")
            }
            Face::Symbol => include_str!("../../../../../assets/afm/Symbol.afm"),
            Face::ZapfDingbats => include_str!("../../../../../assets/afm/ZapfDingbats.afm"),
        }
    }

    /// Whether the face is Symbol or ZapfDingbats, whose glyphs no Latin
    /// encoding names.
    fn is_symbolic(self) -> bool {
        matches!(self, Face::Symbol | Face::ZapfDingbats)
    }

    /// The face's [`UNSETTLED`] glyphs: none for Courier, whose glyphs
    /// renderers all draw 600 wide, or for the symbolic faces.
    fn unsettled(self) -> &'static [&'static str] {
        match self {
            Face::Helvetica
            | Face::HelveticaBold
            | Face::HelveticaOblique
            | Face::HelveticaBoldOblique
            | Face::TimesRoman
            | Face::TimesBold
            | Face::TimesItalic
            | Face::TimesBoldItalic => UNSETTLED,
            _ => &[],
        }
    }

    /// The face's metrics, read from its AFM on first use.
    fn metrics(self) -> &'static Metrics {
        static FACES: [OnceLock<Metrics>; 14] = [const { OnceLock::new() }; 14];
        FACES[self as usize].get_or_init(|| Metrics::parse(self.afm()))
    }

    /// The face a `/BaseFont` names, by its own name or an alias. A subset
    /// prefix (`ABCDEF+`, 9.9.2) is ignored.
    fn named(base_font: &[u8]) -> Option<Face> {
        let name = match base_font.split_at_checked(7) {
            Some(([prefix @ .., b'+'], rest)) if prefix.iter().all(u8::is_ascii_uppercase) => rest,
            _ => base_font,
        };
        Some(match name {
            b"Courier" | b"CourierNew" | b"CourierNewPSMT" => Face::Courier,
            b"Courier-Bold" | b"Courier,Bold" | b"CourierNew,Bold" | b"CourierNewPS-BoldMT" => {
                Face::CourierBold
            }
            b"Courier-Oblique"
            | b"Courier,Italic"
            | b"CourierNew,Italic"
            | b"CourierNewPS-ItalicMT" => Face::CourierOblique,
            b"Courier-BoldOblique"
            | b"Courier,BoldItalic"
            | b"CourierNew,BoldItalic"
            | b"CourierNewPS-BoldItalicMT" => Face::CourierBoldOblique,
            b"Helvetica" | b"Arial" | b"ArialMT" => Face::Helvetica,
            b"Helvetica-Bold" | b"Helvetica,Bold" | b"Arial,Bold" | b"Arial-BoldMT" => {
                Face::HelveticaBold
            }
            b"Helvetica-Oblique" | b"Helvetica,Italic" | b"Arial,Italic" | b"Arial-ItalicMT" => {
                Face::HelveticaOblique
            }
            b"Helvetica-BoldOblique"
            | b"Helvetica,BoldItalic"
            | b"Arial,BoldItalic"
            | b"Arial-BoldItalicMT" => Face::HelveticaBoldOblique,
            b"Times-Roman" | b"Times" | b"TimesNewRoman" | b"TimesNewRomanPSMT" => Face::TimesRoman,
            b"Times-Bold" | b"Times,Bold" | b"TimesNewRoman,Bold" | b"TimesNewRomanPS-BoldMT" => {
                Face::TimesBold
            }
            b"Times-Italic"
            | b"Times,Italic"
            | b"TimesNewRoman,Italic"
            | b"TimesNewRomanPS-ItalicMT" => Face::TimesItalic,
            b"Times-BoldItalic"
            | b"Times,BoldItalic"
            | b"TimesNewRoman,BoldItalic"
            | b"TimesNewRomanPS-BoldItalicMT" => Face::TimesBoldItalic,
            b"Symbol" | b"Symbol,Bold" | b"Symbol,Italic" | b"Symbol,BoldItalic" => Face::Symbol,
            b"ZapfDingbats" => Face::ZapfDingbats,
            _ => return None,
        })
    }
}

/// What a face's AFM gives: its built-in encoding, each glyph's width, and
/// its vertical extent, in thousandths of an em.
#[derive(Debug)]
struct Metrics {
    /// The glyph each code selects under the face's built-in encoding.
    built_in: GlyphNames<'static>,
    /// Each glyph's advance width.
    widths: HashMap<&'static str, f64>,
    cap_height: Option<f64>,
    ascender: Option<f64>,
    descender: Option<f64>,
}

impl Metrics {
    /// Read the AFM `afm`: each character metrics line (`C code ; WX width ;
    /// N name ; …`), and the `CapHeight`, `Ascender` and `Descender`
    /// header keys. Kerning and bounding boxes are not read.
    fn parse(afm: &'static str) -> Metrics {
        let mut built_in = [None; 256];
        let mut widths = HashMap::new();
        let (mut cap_height, mut ascender, mut descender) = (None, None, None);
        for line in afm.lines() {
            if let Some(fields) = line.strip_prefix("C ") {
                let mut code = None;
                let mut width = None;
                let mut name = None;
                for (i, field) in fields.split(';').map(str::trim).enumerate() {
                    if i == 0 {
                        code = field.parse::<i32>().ok();
                    } else if let Some(value) = field.strip_prefix("WX ") {
                        width = value.trim().parse::<f64>().ok();
                    } else if let Some(value) = field.strip_prefix("N ") {
                        name = Some(value.trim());
                    }
                }
                let (Some(code), Some(width), Some(name)) = (code, width, name) else {
                    continue;
                };
                widths.insert(name, width);
                if let Some(slot) = usize::try_from(code).ok().and_then(|c| built_in.get_mut(c)) {
                    *slot = Some(name);
                }
                continue;
            }
            let mut words = line.split_whitespace();
            let value =
                |words: &mut SplitWhitespace<'_>| words.next().and_then(|v| v.parse::<f64>().ok());
            match words.next() {
                Some("CapHeight") => cap_height = value(&mut words),
                Some("Ascender") => ascender = value(&mut words),
                Some("Descender") => descender = value(&mut words),
                _ => {}
            }
        }
        Metrics {
            built_in,
            widths,
            cap_height,
            ascender,
            descender,
        }
    }
}

/// A simple font that names a standard face and embeds no font program, so
/// a reader supplies the face's metrics.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Standard {
    face: Face,
}

impl Standard {
    /// The standard face `font` names, when it is a Type1 or TrueType font
    /// that embeds no font program.
    pub(crate) fn of(doc: &Document, font: &Dictionary) -> Option<Standard> {
        if !matches!(font.name(doc, b"Subtype"), Some(b"Type1" | b"TrueType")) {
            return None;
        }
        let embeds = font
            .get(b"FontDescriptor")
            .ok()
            .and_then(|descriptor| doc.resolve_dict(descriptor))
            .is_some_and(|descriptor| {
                [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
                    .iter()
                    .any(|key| descriptor.has(key))
            });
        if embeds {
            return None;
        }
        let face = Face::named(font.name(doc, b"BaseFont")?)?;
        Some(Standard { face })
    }

    /// The advance width of each code of `font`, in thousandths of an em:
    /// its glyph's width in the face, `None` where the code has no settled
    /// width. `None` overall when renderers read the font's encoding
    /// differently throughout.
    pub(crate) fn widths(self, doc: &Document, font: &Dictionary) -> Option<[Option<f64>; 256]> {
        let metrics = self.face.metrics();
        let unsettled = self.face.unsettled();
        let names = glyph_names(doc, font, self.face)?;
        Some(array::from_fn(|code| {
            let name = names[code]?;
            if unsettled.contains(&name) {
                return None;
            }
            metrics.widths.get(name).copied()
        }))
    }

    /// The face's cap height, in thousandths of an em.
    pub(crate) fn cap_height(self) -> Option<f64> {
        self.face.metrics().cap_height
    }

    /// The face's ascender, in thousandths of an em.
    pub(crate) fn ascender(self) -> Option<f64> {
        self.face.metrics().ascender
    }

    /// The face's descender, in thousandths of an em.
    pub(crate) fn descender(self) -> Option<f64> {
        self.face.metrics().descender
    }
}

/// The glyph name each code of the simple `font` in `face` selects (9.6.5),
/// as PDFium and pdf.js select it: its encoding's base encoding, with any
/// `/Differences` applied.
///
/// The base is `WinAnsiEncoding` or `MacRomanEncoding` when the encoding
/// names one, and otherwise the [`default`](default_encoding): Symbol and
/// ZapfDingbats always read by their own. `None` when renderers read the
/// encoding differently throughout: an `/Encoding` neither a name nor a
/// dictionary, a `MacExpertEncoding` base (pdf.js reads it, PDFium reads
/// `WinAnsiEncoding` for it), or a `/Differences` they apply differently.
fn glyph_names<'a>(doc: &'a Document, font: &'a Dictionary, face: Face) -> Option<GlyphNames<'a>> {
    let default = default_encoding(doc, font, face);
    let Ok(encoding) = font.get(b"Encoding") else {
        return Some(default);
    };
    let (base, differences) = match doc.resolve(encoding)? {
        Object::Name(name) => (Some(name.as_slice()), None),
        Object::Dictionary(dict) => {
            let base = dict
                .get(b"BaseEncoding")
                .ok()
                .and_then(|base| doc.resolve(base))
                .and_then(|base| base.as_name().ok());
            (base, dict.get(b"Differences").ok())
        }
        _ => return None,
    };
    let mut names: GlyphNames<'a> = match base {
        _ if face.is_symbolic() => default,
        Some(b"WinAnsiEncoding") => WIN_ANSI,
        Some(b"MacRomanEncoding") => MAC_ROMAN,
        Some(b"MacExpertEncoding") => return None,
        _ => default,
    };
    if let Some(differences) = differences {
        apply_differences(doc, doc.resolve_array(differences)?, &mut names)?;
    }
    Some(names)
}

/// The base encoding `font` in `face` reads by when it names none it can:
/// a symbolic face's built-in one; else, by its font descriptor's flags,
/// `StandardEncoding` for a font flagged nonsymbolic or a Type1 font flagged
/// neither way, and `WinAnsiEncoding` for a TrueType font flagged neither
/// way. For a font flagged symbolic only, PDFium reads `WinAnsiEncoding` and
/// pdf.js `MacRomanEncoding`, so only the codes the two name alike have a
/// glyph.
fn default_encoding(doc: &Document, font: &Dictionary, face: Face) -> GlyphNames<'static> {
    let built_in = face.metrics().built_in;
    if face.is_symbolic() {
        return built_in;
    }
    let flags = font
        .get(b"FontDescriptor")
        .ok()
        .and_then(|descriptor| doc.resolve_dict(descriptor))
        .and_then(|descriptor| doc.resolve_number(descriptor.get(b"Flags").ok()?))
        .map_or(0, |flags| flags as i64);
    if flags & NONSYMBOLIC != 0 {
        built_in
    } else if flags & SYMBOLIC != 0 {
        array::from_fn(|code| WIN_ANSI[code].filter(|_| WIN_ANSI[code] == MAC_ROMAN[code]))
    } else if font.is_subtype(doc, b"TrueType") {
        WIN_ANSI
    } else {
        built_in
    }
}

/// Apply the `/Differences` array `entries` to `names`: each integer is the
/// code the names after it start at (9.6.5.1), and names before any integer
/// start at code 0. A name for a code past 255 selects nothing. `None` for an
/// entry renderers read differently: a negative code, or one neither an
/// integer nor a name.
fn apply_differences<'a>(
    doc: &'a Document,
    entries: &'a [Object],
    names: &mut GlyphNames<'a>,
) -> Option<()> {
    let mut code = 0_usize;
    for entry in entries {
        match doc.resolve(entry)? {
            Object::Integer(start) => code = usize::try_from(*start).ok()?,
            Object::Name(name) => {
                if let Some(slot) = names.get_mut(code) {
                    *slot = str::from_utf8(name).ok();
                }
                code = code.saturating_add(1);
            }
            _ => return None,
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    /// A non-embedded simple font of `subtype` named `base_font`, with the
    /// extra entries `extra`.
    fn font(subtype: &str, base_font: &str, extra: Dictionary) -> Dictionary {
        let mut font = dictionary! {
            "Type" => "Font",
            "Subtype" => subtype,
            "BaseFont" => base_font,
        };
        font.extend(&extra);
        font
    }

    fn widths(font: &Dictionary) -> Option<[Option<f64>; 256]> {
        let doc = Document::with_version("1.4");
        Standard::of(&doc, font)?.widths(&doc, font)
    }

    /// Every face's AFM reads whole: the glyph counts Adobe's files hold,
    /// and the vertical extent the Latin faces declare (the symbolic ones
    /// declare none).
    #[test]
    fn every_afm_reads() {
        for face in Face::ALL {
            let metrics = face.metrics();
            let (glyphs, extent) = match face {
                Face::Symbol => (190, false),
                Face::ZapfDingbats => (202, false),
                _ => (315, true),
            };
            assert_eq!(metrics.widths.len(), glyphs, "{face:?}");
            assert_eq!(metrics.cap_height.is_some(), extent, "{face:?}");
            assert_eq!(metrics.descender.is_some(), extent, "{face:?}");
        }
        let helvetica = Face::Helvetica.metrics();
        assert_eq!(
            (helvetica.cap_height, helvetica.descender),
            (Some(718.0), Some(-207.0))
        );
    }

    /// The Latin faces' built-in encoding is the standard encoding.
    #[test]
    fn latin_faces_share_the_standard_encoding() {
        let standard = Face::Helvetica.metrics().built_in;
        for face in &Face::ALL[..12] {
            assert_eq!(face.metrics().built_in, standard, "{face:?}");
        }
        assert_eq!(standard[usize::from(b'A')], Some("A"));
        assert_eq!(standard[0x27], Some("quoteright"));
    }

    /// Helvetica's widths reach codes through each encoding by glyph name.
    #[test]
    fn widths_follow_the_encoding() {
        let built_in = widths(&font("Type1", "Helvetica", Dictionary::new())).unwrap();
        assert_eq!(built_in[usize::from(b'A')], Some(667.0));
        assert_eq!(built_in[usize::from(b' ')], Some(278.0));
        assert_eq!(built_in[0x27], Some(222.0), "quoteright");
        assert_eq!(built_in[0x80], None, "no glyph");

        let named = |encoding: &str| {
            widths(&font(
                "Type1",
                "Helvetica",
                dictionary! { "Encoding" => encoding },
            ))
            .unwrap()
        };
        let win_ansi = named("WinAnsiEncoding");
        assert_eq!(win_ansi[0x27], Some(191.0), "quotesingle");
        assert_eq!(win_ansi[0xE9], Some(556.0), "eacute");
        assert_eq!(win_ansi[0xA0], Some(278.0), "space");
        assert_eq!(win_ansi[0xAD], Some(333.0), "hyphen");
        assert_eq!(win_ansi[0x81], Some(350.0), "unused: bullet");
        assert_eq!(win_ansi[0x7F], Some(350.0), "unused: bullet");

        let mac_roman = named("MacRomanEncoding");
        assert_eq!(mac_roman[0xCA], Some(278.0), "space");
        assert_eq!(mac_roman[0x8E], Some(556.0), "eacute");
        assert_eq!(mac_roman[0xAD], Some(549.0), "Mac OS: notequal");
        assert_eq!(mac_roman[0xB0], None, "Mac OS: infinity, not in the face");

        let standard = widths(&font(
            "Type1",
            "Times-Roman",
            dictionary! { "Encoding" => "StandardEncoding" },
        ))
        .unwrap();
        assert_eq!(standard[0x27], Some(333.0), "quoteright");
    }

    /// A glyph renderers size differently has no width in the Helvetica and
    /// Times faces, however the font names them; Courier's are all settled.
    #[test]
    fn unsettled_glyphs_have_no_width() {
        let win_ansi = |subtype: &str, base_font: &str| {
            widths(&font(
                subtype,
                base_font,
                dictionary! { "Encoding" => "WinAnsiEncoding" },
            ))
            .unwrap()
        };
        for base_font in ["Helvetica", "Times-Bold"] {
            let face = win_ansi("Type1", base_font);
            assert_eq!(face[0xF7], None, "{base_font}: divide");
            assert_eq!(face[0xAF], None, "{base_font}: macron");
        }
        assert_eq!(win_ansi("Type1", "Courier")[0xF7], Some(600.0));
        assert_eq!(
            win_ansi("TrueType", "Arial"),
            win_ansi("Type1", "Helvetica")
        );
        assert_eq!(
            win_ansi("TrueType", "TimesNewRoman,Bold"),
            win_ansi("Type1", "Times-Bold")
        );
    }

    /// `/Differences` renames codes over the base encoding, from code 0
    /// before any code is given, and past code 255 selects nothing.
    #[test]
    fn differences_rename_codes() {
        let differences = |base_font: &str, encoding: Dictionary| {
            widths(&font(
                "Type1",
                base_font,
                dictionary! { "Encoding" => encoding },
            ))
            .unwrap()
        };
        let courier = differences(
            "Courier-Bold",
            dictionary! {
                "BaseEncoding" => "WinAnsiEncoding",
                "Differences" => vec![
                    65.into(), "W".into(), "i".into(),
                    200.into(), "nosuchglyph".into(),
                ],
            },
        );
        assert_eq!(courier[65], Some(600.0));
        assert_eq!(courier[66], Some(600.0));
        assert_eq!(courier[200], None, "a glyph the face lacks");

        let helvetica = differences(
            "Helvetica",
            dictionary! { "Differences" => vec![65.into(), "W".into()] },
        );
        assert_eq!(helvetica[65], Some(944.0), "W");
        assert_eq!(helvetica[66], Some(667.0), "B, from the built-in base");

        let leading = differences(
            "Helvetica",
            dictionary! { "Differences" => vec!["W".into()] },
        );
        assert_eq!(leading[0], Some(944.0), "a name before any code");

        let past = differences(
            "Helvetica",
            dictionary! {
                "Differences" => vec![
                    300.into(), "A".into(),
                    255.into(), "A".into(), "W".into(),
                    65.into(), "W".into(),
                ],
            },
        );
        assert_eq!(past[255], Some(667.0), "A");
        assert_eq!(past[65], Some(944.0), "W, after codes past 255");
    }

    /// An encoding renderers read differently gives no widths at all.
    #[test]
    fn an_unsettled_encoding_gives_none() {
        for encoding in [
            Object::from("MacExpertEncoding"),
            Object::from(3),
            Object::Dictionary(dictionary! { "BaseEncoding" => "MacExpertEncoding" }),
            Object::Dictionary(dictionary! { "Differences" => vec![(-1).into(), "A".into()] }),
            Object::Dictionary(dictionary! { "Differences" => vec![65.into(), 1.5.into()] }),
            Object::Dictionary(dictionary! {
                "Differences" => vec![65.into(), Object::string_literal("x"), "W".into()],
            }),
            Object::Dictionary(dictionary! { "Differences" => 3 }),
        ] {
            let font = font(
                "Type1",
                "Helvetica",
                dictionary! { "Encoding" => encoding.clone() },
            );
            assert!(widths(&font).is_none(), "{encoding:?}");
        }
    }

    /// A base encoding renderers don't know, or that isn't a name, leaves the
    /// font's default in place: the standard encoding for a Type1 font,
    /// WinAnsi for a TrueType font, unless its flags say otherwise.
    #[test]
    fn the_default_encoding_follows_subtype_and_flags() {
        let quote = |subtype: &str, base_font: &str, extra: Dictionary| {
            widths(&font(subtype, base_font, extra)).unwrap()[0x27]
        };
        let flags =
            |flags: i64| dictionary! { "FontDescriptor" => dictionary! { "Flags" => flags } };
        let quoteright = Some(222.0);
        let quotesingle = Some(191.0);

        assert_eq!(quote("Type1", "Helvetica", Dictionary::new()), quoteright);
        assert_eq!(quote("TrueType", "Arial", Dictionary::new()), quotesingle);
        assert_eq!(quote("TrueType", "Arial", flags(NONSYMBOLIC)), quoteright);
        for encoding in [
            Object::from("Foo"),
            Object::from("StandardEncoding"),
            Object::Dictionary(dictionary! { "BaseEncoding" => 1 }),
            Object::Dictionary(dictionary! { "BaseEncoding" => "Foo" }),
        ] {
            let extra = dictionary! { "Encoding" => encoding.clone() };
            assert_eq!(
                quote("Type1", "Helvetica", extra.clone()),
                quoteright,
                "{encoding:?}"
            );
            assert_eq!(
                quote("TrueType", "Arial", extra),
                quotesingle,
                "{encoding:?}"
            );
        }

        // Flagged symbolic only: WinAnsi and MacRoman agree on ASCII, not
        // above it.
        let symbolic = widths(&font("Type1", "Helvetica", flags(SYMBOLIC))).unwrap();
        assert_eq!(symbolic[0x27], quotesingle);
        assert_eq!(symbolic[usize::from(b'A')], Some(667.0));
        assert_eq!(symbolic[0x80], None, "Euro or Adieresis");
    }

    /// Symbol and ZapfDingbats read through their own built-in encodings,
    /// whatever base encoding a font names, with its `/Differences`.
    #[test]
    fn symbolic_faces_use_their_own_encoding() {
        let symbol = widths(&font("Type1", "Symbol", Dictionary::new())).unwrap();
        assert_eq!(symbol[usize::from(b'a')], Some(631.0), "alpha");
        let named = font(
            "Type1",
            "Symbol",
            dictionary! { "Encoding" => "WinAnsiEncoding" },
        );
        assert_eq!(widths(&named).unwrap()[usize::from(b'a')], Some(631.0));

        let dingbats = widths(&font("Type1", "ZapfDingbats", Dictionary::new())).unwrap();
        assert_eq!(dingbats[0x21], Some(974.0), "a1");
        let differences = font(
            "Type1",
            "ZapfDingbats",
            dictionary! { "Encoding" => dictionary! {
                "BaseEncoding" => "WinAnsiEncoding",
                "Differences" => vec![33.into(), "a2".into()],
            } },
        );
        assert_eq!(widths(&differences).unwrap()[0x21], Some(961.0), "a2");
    }

    /// Aliases and subset prefixes name the standard faces.
    #[test]
    fn aliases_and_subsets_name_faces() {
        let courier_new = widths(&font("TrueType", "CourierNewPSMT", Dictionary::new())).unwrap();
        assert_eq!(courier_new[usize::from(b'A')], Some(600.0));

        let subset = widths(&font("Type1", "ABCDEF+Times-Italic", Dictionary::new())).unwrap();
        assert_eq!(subset[usize::from(b'A')], Some(611.0));
        for name in [
            "abcdef+Times-Italic",
            "ABCDE+Times-Italic",
            "Times-Italic-X",
        ] {
            assert!(
                widths(&font("Type1", name, Dictionary::new())).is_none(),
                "{name}"
            );
        }
    }

    /// A font that embeds its program, or isn't a Type1 or TrueType font,
    /// isn't given the standard metrics.
    #[test]
    fn embedded_and_other_fonts_are_not_standard() {
        let doc = Document::with_version("1.4");
        let descriptor = dictionary! { "FontFile2" => Object::Null };
        let embedded = font(
            "TrueType",
            "Arial",
            dictionary! { "FontDescriptor" => descriptor },
        );
        assert!(Standard::of(&doc, &embedded).is_none());
        let type3 = font("Type3", "Helvetica", Dictionary::new());
        assert!(Standard::of(&doc, &type3).is_none());
        let plain = font(
            "Type1",
            "Helvetica",
            dictionary! { "FontDescriptor" => dictionary! {} },
        );
        assert!(Standard::of(&doc, &plain).is_some());
    }
}
