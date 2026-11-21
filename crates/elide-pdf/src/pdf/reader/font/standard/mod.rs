//! The standard 14 fonts (9.6.2.2): the glyphs, widths and vertical metrics a
//! reader supplies for a simple font that names one and embeds no font
//! program.
//!
//! The metrics are Adobe's Core14 AFM files (`assets/afm`). A font's codes
//! reach its glyphs by name: its encoding names the glyph each code selects
//! (9.6.5), and the AFM gives each glyph name its width. A font may name a
//! face by an alternative name (`Arial`, `TimesNewRoman,Bold`, `CourierNew`),
//! which renderers draw with the face's metrics.
//!
//! Such a font carries no widths of its own, so its glyphs are as wide as the
//! renderer showing it makes them, and renderers differ on a few. The rebuild
//! makes them definite: it writes the glyphs and widths read here into the
//! font ([`Standard::names`], [`Standard::widths`]), so every viewer selects
//! and places the same glyphs. Where PDFium and pdf.js select a glyph alike,
//! that is the glyph read, even where they depart from the letter of Annex D;
//! where they differ, the spec's reading is. Every glyph is as wide as its AFM
//! says, and a code that selects no glyph the face has is 0 wide.

use std::array;
use std::collections::HashMap;
use std::str::SplitWhitespace;
use std::sync::OnceLock;

use lopdf::{Dictionary, Document, Object, dictionary};

use super::encoding::{self, BaseEncoding, Encoding, GlyphNames, NONSYMBOLIC, SYMBOLIC};
use super::subset_free;
use crate::pdf::reader::{DictionaryExt, DocumentExt};

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

    /// The face's metrics, read from its AFM on first use.
    fn metrics(self) -> &'static Metrics {
        static FACES: [OnceLock<Metrics>; 14] = [const { OnceLock::new() }; 14];
        FACES[self as usize].get_or_init(|| Metrics::parse(self.afm()))
    }

    /// The face a `/BaseFont` names, by its own name or an alias. A subset
    /// prefix (`ABCDEF+`, 9.9.2) is ignored.
    fn named(base_font: &[u8]) -> Option<Face> {
        Some(match subset_free(base_font) {
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
    /// `FontBBox`: the glyphs' joint bounding box.
    bbox: [f64; 4],
    italic_angle: f64,
    /// `StdVW`: the dominant vertical stem width.
    stem_v: f64,
}

impl Metrics {
    /// Read the AFM `afm`: each character metrics line (`C code ; WX width ;
    /// N name ; …`), and the `CapHeight`, `Ascender` and `Descender`
    /// header keys. Kerning and bounding boxes are not read.
    fn parse(afm: &'static str) -> Metrics {
        let mut built_in = [None; 256];
        let mut widths = HashMap::new();
        let (mut cap_height, mut ascender, mut descender) = (None, None, None);
        let (mut bbox, mut italic_angle, mut stem_v) = ([0.0; 4], 0.0, 0.0);
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
                Some("FontBBox") => {
                    for corner in &mut bbox {
                        *corner = value(&mut words).unwrap_or_default();
                    }
                }
                Some("ItalicAngle") => italic_angle = value(&mut words).unwrap_or_default(),
                Some("StdVW") => stem_v = value(&mut words).unwrap_or_default(),
                _ => {}
            }
        }
        Metrics {
            built_in,
            widths,
            cap_height,
            ascender,
            descender,
            bbox,
            italic_angle,
            stem_v,
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

    /// The glyph each code of `font` selects, `None` for a code that
    /// selects none.
    pub(crate) fn names<'a>(self, doc: &'a Document, font: &'a Dictionary) -> GlyphNames<'a> {
        glyph_names(doc, font, self.face)
    }

    /// The text each code of `font` stands for, from the glyph it selects
    /// (Adobe Glyph List), `None` for a code whose glyph spells none.
    pub(crate) fn text(self, doc: &Document, font: &Dictionary) -> Box<[Option<String>; 256]> {
        let dingbats = self.face == Face::ZapfDingbats;
        let names = glyph_names(doc, font, self.face);
        Box::new(array::from_fn(|code| {
            encoding::text(names[code]?, dingbats)
        }))
    }

    /// The advance width of each code of `font`, in thousandths of an em:
    /// its glyph's width in the face, 0 for a code that selects no glyph the
    /// face has.
    pub(crate) fn widths(self, doc: &Document, font: &Dictionary) -> [f64; 256] {
        let metrics = self.face.metrics();
        let names = glyph_names(doc, font, self.face);
        array::from_fn(|code| {
            names[code]
                .and_then(|name| metrics.widths.get(name).copied())
                .unwrap_or(0.0)
        })
    }

    /// The font descriptor `/Flags` that make every reader take `names` as
    /// they are: Symbolic for Symbol and ZapfDingbats, Nonsymbolic for the
    /// others, as an explicit `/Encoding` is read only for a nonsymbolic
    /// TrueType font (9.6.5.4). `flags`'s other bits are kept.
    pub(crate) fn flags(self, flags: i64) -> i64 {
        if self.face.is_symbolic() {
            (flags | SYMBOLIC) & !NONSYMBOLIC
        } else {
            (flags | NONSYMBOLIC) & !SYMBOLIC
        }
    }

    /// A font descriptor (9.8.1) for the face, named `name`, with `flags`
    /// and the face's metrics: what a font that names the face and has none
    /// is given, so its flags can be set (see [`flags`](Self::flags)). A
    /// symbolic face, which declares no cap height or descender, takes its
    /// bounding box's.
    pub(crate) fn descriptor(self, name: &[u8], flags: i64) -> Dictionary {
        let metrics = self.face.metrics();
        let [left, bottom, right, top] = metrics.bbox;
        dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => Object::Name(name.to_vec()),
            "Flags" => flags,
            "FontBBox" => vec![left.into(), bottom.into(), right.into(), top.into()],
            "ItalicAngle" => metrics.italic_angle,
            "Ascent" => metrics.ascender.unwrap_or(top),
            "Descent" => metrics.descender.unwrap_or(bottom),
            "CapHeight" => metrics.cap_height.unwrap_or(top),
            "StemV" => metrics.stem_v,
        }
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

/// The glyph name each code of the simple `font` in `face` selects (9.6.5):
/// its encoding's base encoding, with any `/Differences` applied.
///
/// The base is `WinAnsiEncoding`, `MacRomanEncoding` or `MacExpertEncoding`
/// when the encoding names one, and otherwise the
/// [`default`](default_encoding); Symbol and ZapfDingbats always read by their
/// own, as PDFium and pdf.js read them.
fn glyph_names<'a>(doc: &'a Document, font: &'a Dictionary, face: Face) -> GlyphNames<'a> {
    let encoding = Encoding::of(doc, font);
    let mut names = match encoding.base() {
        Some(base) if !face.is_symbolic() => base.names(),
        _ => default_encoding(doc, font, face),
    };
    encoding.apply(doc, &mut names);
    names
}

/// The base encoding `font` in `face` reads by when it names none it can: a
/// symbolic face's built-in one; else, by its font descriptor's flags, the
/// standard encoding (the face's built-in one) for a Type1 font or one flagged
/// nonsymbolic, `WinAnsiEncoding` for a TrueType font flagged neither way, as
/// PDFium and pdf.js read it, and the Mac OS Roman encoding for a TrueType
/// font flagged symbolic only, as the spec reads it (9.6.5.4).
fn default_encoding(doc: &Document, font: &Dictionary, face: Face) -> GlyphNames<'static> {
    let built_in = face.metrics().built_in;
    if face.is_symbolic() || !font.is_subtype(doc, b"TrueType") {
        return built_in;
    }
    let flags = encoding::flags(doc, font);
    if flags & NONSYMBOLIC != 0 {
        built_in
    } else if flags & SYMBOLIC != 0 {
        BaseEncoding::MacOsRoman.names()
    } else {
        BaseEncoding::WinAnsi.names()
    }
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

    fn widths(font: &Dictionary) -> Option<[f64; 256]> {
        let doc = Document::with_version("1.4");
        Some(Standard::of(&doc, font)?.widths(&doc, font))
    }

    /// The widths of the Type1 face `base_font` under the encoding `encoding`.
    fn encoded(base_font: &str, encoding: impl Into<Object>) -> [f64; 256] {
        widths(&font(
            "Type1",
            base_font,
            dictionary! { "Encoding" => encoding.into() },
        ))
        .unwrap()
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
        let standard = BaseEncoding::Standard.names();
        for face in &Face::ALL[..12] {
            assert_eq!(face.metrics().built_in, standard, "{face:?}");
        }
        assert_eq!(standard[usize::from(b'A')], Some("A"));
        assert_eq!(standard[0x27], Some("quoteright"));
    }

    /// Helvetica's widths reach codes through each encoding by glyph name; a
    /// code that selects no glyph the face has is 0 wide.
    #[test]
    fn widths_follow_the_encoding() {
        let built_in = widths(&font("Type1", "Helvetica", Dictionary::new())).unwrap();
        assert_eq!(built_in[usize::from(b'A')], 667.0);
        assert_eq!(built_in[usize::from(b' ')], 278.0);
        assert_eq!(built_in[0x27], 222.0, "quoteright");
        assert_eq!(built_in[0x80], 0.0, "no glyph");

        let win_ansi = encoded("Helvetica", "WinAnsiEncoding");
        assert_eq!(win_ansi[0x27], 191.0, "quotesingle");
        assert_eq!(win_ansi[0xE9], 556.0, "eacute");
        assert_eq!(win_ansi[0xA0], 278.0, "space");
        assert_eq!(win_ansi[0xAD], 333.0, "hyphen");
        assert_eq!(win_ansi[0x81], 350.0, "unused: bullet");
        assert_eq!(win_ansi[0x7F], 350.0, "unused: bullet");

        let mac_roman = encoded("Helvetica", "MacRomanEncoding");
        assert_eq!(mac_roman[0xCA], 278.0, "space");
        assert_eq!(mac_roman[0x8E], 556.0, "eacute");
        assert_eq!(mac_roman[0xAD], 549.0, "Mac OS: notequal");
        assert_eq!(mac_roman[0xB0], 0.0, "Mac OS: infinity, not in the face");

        let expert = encoded("Helvetica", "MacExpertEncoding");
        assert_eq!(expert[0x2C], 278.0, "comma");
        assert_eq!(expert[0x30], 0.0, "zerooldstyle, not in the face");

        assert_eq!(
            encoded("Times-Roman", "StandardEncoding")[0x27],
            333.0,
            "quoteright"
        );
    }

    /// Every glyph is as wide as Adobe's metrics say, including those a
    /// renderer sizes otherwise, and an alias reads as its face.
    #[test]
    fn glyphs_take_adobes_widths() {
        let helvetica = encoded("Helvetica", "WinAnsiEncoding");
        assert_eq!(helvetica[0xF7], 584.0, "divide");
        assert_eq!(helvetica[0xAF], 333.0, "macron");
        let arial = widths(&font(
            "TrueType",
            "Arial",
            dictionary! { "Encoding" => "WinAnsiEncoding" },
        ))
        .unwrap();
        assert_eq!(arial, helvetica);
        assert_eq!(encoded("Courier", "WinAnsiEncoding")[0xF7], 600.0);
    }

    /// `/Differences` renames codes over the base encoding, from code 0
    /// before any code is given; past code 255 selects nothing.
    #[test]
    fn differences_rename_codes() {
        let courier = encoded(
            "Courier-Bold",
            dictionary! {
                "BaseEncoding" => "WinAnsiEncoding",
                "Differences" => vec![
                    65.into(), "W".into(), "i".into(),
                    200.into(), "nosuchglyph".into(),
                ],
            },
        );
        assert_eq!(courier[65], 600.0);
        assert_eq!(courier[66], 600.0);
        assert_eq!(courier[200], 0.0, "a glyph the face lacks");

        let helvetica = encoded(
            "Helvetica",
            dictionary! { "Differences" => vec![65.into(), "W".into()] },
        );
        assert_eq!(helvetica[65], 944.0, "W");
        assert_eq!(helvetica[66], 667.0, "B, from the built-in base");

        let leading = encoded(
            "Helvetica",
            dictionary! { "Differences" => vec!["W".into()] },
        );
        assert_eq!(leading[0], 944.0, "a name before any code");

        let past = encoded(
            "Helvetica",
            dictionary! {
                "Differences" => vec![
                    300.into(), "A".into(),
                    255.into(), "A".into(), "W".into(),
                    65.into(), "W".into(),
                ],
            },
        );
        assert_eq!(past[255], 667.0, "A");
        assert_eq!(past[65], 944.0, "W, after codes past 255");
    }

    /// Entries the spec gives no reading of are skipped: the names after a
    /// negative code, and entries neither an integer nor a name.
    #[test]
    fn malformed_differences_are_skipped() {
        let negative = encoded(
            "Helvetica",
            dictionary! { "Differences" => vec![(-1).into(), "W".into(), "W".into(), 66.into(), "W".into()] },
        );
        assert_eq!(negative[0], 0.0, "no name reaches code 0");
        assert_eq!(negative[66], 944.0);

        for entry in [Object::Real(1.5), Object::string_literal("x")] {
            let skipped = encoded(
                "Helvetica",
                dictionary! { "Differences" => vec![65.into(), entry.clone(), "W".into()] },
            );
            assert_eq!(skipped[65], 944.0, "{entry:?}");
        }
    }

    /// An encoding no reader can use leaves the font's default in place.
    #[test]
    fn an_unusable_encoding_is_the_default() {
        for encoding in [
            Object::from(3),
            Object::from("Foo"),
            Object::Dictionary(dictionary! { "BaseEncoding" => 1 }),
            Object::Dictionary(dictionary! { "BaseEncoding" => "Foo" }),
            Object::Dictionary(dictionary! { "Differences" => 3 }),
        ] {
            assert_eq!(
                encoded("Helvetica", encoding.clone())[0x27],
                222.0,
                "{encoding:?}"
            );
        }
    }

    /// With no encoding of its own, a Type1 font reads by the standard
    /// encoding, a TrueType font by WinAnsi, or by Mac OS Roman when flagged
    /// symbolic only, or by the standard encoding when flagged nonsymbolic.
    #[test]
    fn the_default_encoding_follows_subtype_and_flags() {
        let flags =
            |flags: i64| dictionary! { "FontDescriptor" => dictionary! { "Flags" => flags } };
        let quote = |subtype: &str, base_font: &str, extra: Dictionary| {
            widths(&font(subtype, base_font, extra)).unwrap()[0x27]
        };
        assert_eq!(quote("Type1", "Helvetica", Dictionary::new()), 222.0);
        assert_eq!(quote("Type1", "Helvetica", flags(SYMBOLIC)), 222.0);
        assert_eq!(quote("TrueType", "Arial", Dictionary::new()), 191.0);
        assert_eq!(quote("TrueType", "Arial", flags(NONSYMBOLIC)), 222.0);

        let mac_os = widths(&font("TrueType", "Arial", flags(SYMBOLIC))).unwrap();
        assert_eq!(mac_os[0x27], 191.0, "quotesingle");
        assert_eq!(mac_os[0x80], 667.0, "Adieresis");
        assert_eq!(mac_os[0xDB], 556.0, "Euro");
    }

    /// Symbol and ZapfDingbats read through their own built-in encodings,
    /// whatever base encoding a font names, with its `/Differences`.
    #[test]
    fn symbolic_faces_use_their_own_encoding() {
        let symbol = widths(&font("Type1", "Symbol", Dictionary::new())).unwrap();
        assert_eq!(symbol[usize::from(b'a')], 631.0, "alpha");
        assert_eq!(
            encoded("Symbol", "WinAnsiEncoding")[usize::from(b'a')],
            631.0
        );

        let dingbats = widths(&font("Type1", "ZapfDingbats", Dictionary::new())).unwrap();
        assert_eq!(dingbats[0x21], 974.0, "a1");
        let differences = encoded(
            "ZapfDingbats",
            dictionary! {
                "BaseEncoding" => "WinAnsiEncoding",
                "Differences" => vec![33.into(), "a2".into()],
            },
        );
        assert_eq!(differences[0x21], 961.0, "a2");
    }

    /// The flags a pinned font is written with: Nonsymbolic for the Latin
    /// faces, Symbolic for the symbolic ones, other bits kept.
    #[test]
    fn pinned_flags_follow_the_face() {
        let doc = Document::with_version("1.4");
        let helvetica = Standard::of(&doc, &font("Type1", "Helvetica", Dictionary::new())).unwrap();
        assert_eq!(helvetica.flags(SYMBOLIC | 1 << 6), NONSYMBOLIC | 1 << 6);
        let symbol = Standard::of(&doc, &font("Type1", "Symbol", Dictionary::new())).unwrap();
        assert_eq!(symbol.flags(NONSYMBOLIC), SYMBOLIC);
    }

    /// Aliases and subset prefixes name the standard faces.
    #[test]
    fn aliases_and_subsets_name_faces() {
        let courier_new = widths(&font("TrueType", "CourierNewPSMT", Dictionary::new())).unwrap();
        assert_eq!(courier_new[usize::from(b'A')], 600.0);

        let subset = widths(&font("Type1", "ABCDEF+Times-Italic", Dictionary::new())).unwrap();
        assert_eq!(subset[usize::from(b'A')], 611.0);
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
