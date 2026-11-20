//! Embedded font programs (9.9): the advance widths of a simple font that
//! embeds its program and gives no `/Widths`.
//!
//! A TrueType or OpenType program (`/FontFile2`, or `/FontFile3` of subtype
//! `OpenType`) and a bare CFF program (`/FontFile3` of subtype `Type1C`) are
//! read. A Type 1 program (`/FontFile`) is not, so such a font's glyphs stay
//! unplaced.
//!
//! A code reaches its glyph as 9.6.5 lays out. In a Type1 font it reaches it
//! by the glyph name its encoding gives it, or through the program's
//! built-in encoding where the font's encoding names none. In a TrueType font
//! it reaches it through the program's `cmap` (9.6.5.4): by glyph name and
//! Unicode for a nonsymbolic font, by code for a symbolic one, each falling
//! back to the other. A code that reaches no glyph draws `.notdef`, glyph 0,
//! and is as wide. Advances are rounded to whole thousandths of an em, as
//! PDFium rounds a program's.
//!
//! Renderers map codes to glyphs with heuristics of their own where this
//! fails, so the rebuild writes the widths read here into the font as its
//! `/Widths`, and every viewer places the glyphs where the reader does.

use std::array;

use lopdf::{Dictionary, Document, Stream};
use ttf_parser::cmap::Subtable;
use ttf_parser::{Face, GlyphId, PlatformId, cff};

use super::encoding::{self, BaseEncoding, Encoding, NONSYMBOLIC, SYMBOLIC};
use crate::pdf::reader::{DictionaryExt, DocumentExt, StreamExt};

/// The advance width of each code of the simple `font`, in thousandths of an
/// em, from the program it embeds; `None` when it embeds none that is read,
/// the program can't be parsed, or no code reaches a glyph in it.
pub(crate) fn embedded_widths(doc: &Document, font: &Dictionary) -> Option<[f64; 256]> {
    let reach: Reach = match font.name(doc, b"Subtype")? {
        b"TrueType" => truetype_glyphs,
        b"Type1" | b"MMType1" => type1_glyphs,
        _ => return None,
    };
    let descriptor = doc.resolve_dict(font.get(b"FontDescriptor").ok()?)?;
    let (stream, format) = match file(doc, descriptor, b"FontFile2") {
        Some(stream) => (stream, Format::Sfnt),
        None => {
            let stream = file(doc, descriptor, b"FontFile3")?;
            match stream.dict.name(doc, b"Subtype")? {
                b"OpenType" => (stream, Format::Sfnt),
                b"Type1C" => (stream, Format::Cff),
                _ => return None,
            }
        }
    };
    let data = stream.decoded().ok()?;
    let program = match format {
        Format::Sfnt => Program::Sfnt(Box::new(Face::parse(&data, 0).ok()?)),
        Format::Cff => Program::Cff(Box::new(cff::Table::parse(&data)?)),
    };
    let glyphs = reach(doc, font, &program);
    if glyphs.iter().all(Option::is_none) {
        return None;
    }
    let notdef = program.advance(GlyphId(0)).unwrap_or(0.0);
    Some(array::from_fn(|code| {
        glyphs[code]
            .and_then(|glyph| program.advance(glyph))
            .unwrap_or(notdef)
    }))
}

/// How each code of a font reaches its glyph in a program.
type Reach = fn(&Document, &Dictionary, &Program<'_>) -> [Option<GlyphId>; 256];

/// The format of an embedded program that is read.
enum Format {
    /// TrueType or OpenType.
    Sfnt,
    /// Bare CFF.
    Cff,
}

/// The font file `key` of `descriptor`, when it is a stream.
fn file<'a>(doc: &'a Document, descriptor: &'a Dictionary, key: &[u8]) -> Option<&'a Stream> {
    doc.resolve(descriptor.get(key).ok()?)?.as_stream().ok()
}

/// The glyph each code of the Type1 `font` reaches in `program`: by the name
/// its encoding gives the code, else, where the font names no base encoding,
/// through the program's built-in one.
fn type1_glyphs(
    doc: &Document,
    font: &Dictionary,
    program: &Program<'_>,
) -> [Option<GlyphId>; 256] {
    let encoding = Encoding::of(doc, font);
    let base = encoding.base();
    let mut names = base.map_or([None; 256], BaseEncoding::names);
    encoding.apply(doc, &mut names);
    array::from_fn(|code| match names[code] {
        Some(name) => program.by_name(name),
        None if base.is_none() => program.built_in(code as u8),
        None => None,
    })
}

/// The glyph each code of the TrueType `font` reaches in `program`'s `cmap`
/// (9.6.5.4): by glyph name for a font that names `WinAnsiEncoding` or
/// `MacRomanEncoding` or is not flagged symbolic only, else by code, each
/// falling back to the other. A code's name is its encoding's, else the
/// standard encoding's.
fn truetype_glyphs(
    doc: &Document,
    font: &Dictionary,
    program: &Program<'_>,
) -> [Option<GlyphId>; 256] {
    let encoding = Encoding::of(doc, font);
    let flags = encoding::flags(doc, font);
    let symbolic = flags & SYMBOLIC != 0 && flags & NONSYMBOLIC == 0 && !encoding.is_latin();
    let standard = BaseEncoding::Standard.names();
    let mut names = encoding.base().map_or(standard, BaseEncoding::names);
    encoding.apply(doc, &mut names);
    for (name, standard) in names.iter_mut().zip(standard) {
        *name = name.or(standard);
    }
    let unicode = program.cmap(Subtable::is_unicode);
    let mac =
        program.cmap(|table| table.platform_id == PlatformId::Macintosh && table.encoding_id == 0);
    let windows_symbol =
        program.cmap(|table| table.platform_id == PlatformId::Windows && table.encoding_id == 0);

    let by_name = |code: usize| {
        let name = names[code]?;
        let mapped = match (&unicode, &mac) {
            (Some(unicode), _) => glyph(unicode, u32::from(encoding::unicode(name)?)),
            (None, Some(mac)) => {
                let code = BaseEncoding::MacOsRoman.code(name)?;
                glyph(mac, u32::from(code))
            }
            (None, None) => None,
        };
        mapped.or_else(|| program.by_name(name))
    };
    let by_code = |code: usize| {
        let code = code as u32;
        windows_symbol
            .as_ref()
            .and_then(|table| {
                [code, 0xF000 + code, 0xF100 + code, 0xF200 + code]
                    .into_iter()
                    .find_map(|code| glyph(table, code))
            })
            .or_else(|| glyph(mac.as_ref()?, code))
    };
    array::from_fn(|code| {
        if symbolic {
            by_code(code).or_else(|| by_name(code))
        } else {
            by_name(code).or_else(|| by_code(code))
        }
    })
}

/// The glyph `code` reaches through `table`, `None` for `.notdef`.
fn glyph(table: &Subtable<'_>, code: u32) -> Option<GlyphId> {
    table.glyph_index(code).filter(|glyph| glyph.0 != 0)
}

/// A parsed font program.
enum Program<'a> {
    /// A TrueType or OpenType program.
    Sfnt(Box<Face<'a>>),
    /// A bare CFF program.
    Cff(Box<cff::Table<'a>>),
}

impl<'a> Program<'a> {
    /// The glyph named `name`, from an sfnt's `post` or CFF table or the CFF
    /// program's charset.
    fn by_name(&self, name: &str) -> Option<GlyphId> {
        match self {
            Program::Sfnt(face) => face.glyph_index_by_name(name),
            Program::Cff(table) => table.glyph_index_by_name(name),
        }
        .filter(|glyph| glyph.0 != 0)
    }

    /// The glyph `code` reaches through the CFF program's built-in encoding.
    fn built_in(&self, code: u8) -> Option<GlyphId> {
        match self {
            Program::Sfnt(face) => face.tables().cff.as_ref()?.glyph_index(code),
            Program::Cff(table) => table.glyph_index(code),
        }
        .filter(|glyph| glyph.0 != 0)
    }

    /// The first `cmap` subtable of an sfnt that `select` picks.
    fn cmap(&self, select: impl Fn(&Subtable<'a>) -> bool) -> Option<Subtable<'a>> {
        let Program::Sfnt(face) = self else {
            return None;
        };
        face.tables()
            .cmap?
            .subtables
            .into_iter()
            .find(|table| select(table))
    }

    /// The advance width of `glyph`, in whole thousandths of an em, as
    /// renderers round a program's advances.
    fn advance(&self, glyph: GlyphId) -> Option<f64> {
        let advance = match self {
            Program::Sfnt(face) => {
                let em = f64::from(face.units_per_em());
                let advance = f64::from(face.glyph_hor_advance(glyph)?);
                (em > 0.0).then(|| advance * 1000.0 / em)?
            }
            Program::Cff(table) => {
                let width = f64::from(table.glyph_width(glyph)?);
                // The matrix is read as `f32`; its shortest decimal is the
                // value the program wrote, where widening it is not.
                let scale = table.matrix().sx.to_string().parse::<f64>().ok()?;
                width * (scale * 1000.0)
            }
        };
        Some(advance.round())
    }
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::fixtures::{Cff, FontFile, Sfnt, embedding};

    /// The `.notdef`, `A`, `B` and `g.alt` glyphs of an sfnt of 2000 units
    /// to the em, 300, 600, 700 and 800 thousandths of an em wide.
    fn sfnt() -> Sfnt {
        Sfnt::new(
            2000,
            &[(".notdef", 600), ("A", 1200), ("B", 1400), ("g.alt", 1600)],
        )
    }

    /// The widths of the TrueType font embedding `program`, flagged `flags`,
    /// with the font entries `extra`.
    fn truetype(program: &Sfnt, flags: i64, extra: Dictionary) -> Option<[f64; 256]> {
        let mut doc = Document::with_version("1.7");
        let mut font = embedding(
            &mut doc,
            "TrueType",
            FontFile::TrueType,
            program.build(),
            flags,
        );
        font.extend(&extra);
        embedded_widths(&doc, &font)
    }

    /// A Type1 font embedding the CFF `program`, with the font entries `extra`.
    fn type1c(program: &Cff, extra: Dictionary) -> Option<[f64; 256]> {
        let mut doc = Document::with_version("1.7");
        let mut font = embedding(&mut doc, "Type1", FontFile::Cff, program.build(), 4);
        font.extend(&extra);
        embedded_widths(&doc, &font)
    }

    /// A nonsymbolic TrueType font reaches its glyphs by name, through the
    /// Unicode `cmap`, its advances scaled to thousandths of an em; a code
    /// that reaches none is as wide as `.notdef`.
    #[test]
    fn a_nonsymbolic_truetype_font_reads_by_unicode() {
        let program = sfnt().cmap(3, 1, 0x41, &[1, 2]);
        let widths = truetype(
            &program,
            32,
            dictionary! { "Encoding" => "WinAnsiEncoding" },
        )
        .unwrap();
        assert_eq!(widths[0x41], 600.0);
        assert_eq!(widths[0x42], 700.0);
        assert_eq!(widths[0x43], 300.0, ".notdef");
    }

    /// Advances are rounded to whole thousandths of an em.
    #[test]
    fn advances_round_to_whole_thousandths() {
        let program = Sfnt::new(2048, &[(".notdef", 0), ("H", 1479)]).cmap(3, 1, 0x48, &[1]);
        let widths = truetype(&program, 32, Dictionary::new()).unwrap();
        assert_eq!(widths[0x48], 722.0);
    }

    /// `/Differences` rename codes, and a name the Unicode `cmap` can't
    /// reach is found by the program's own glyph names.
    #[test]
    fn differences_reach_glyphs_by_name() {
        let program = sfnt().cmap(3, 1, 0x41, &[1, 2]);
        let encoding = dictionary! {
            "Differences" => vec![0x41.into(), "B".into(), "g.alt".into()],
        };
        let widths = truetype(&program, 32, dictionary! { "Encoding" => encoding }).unwrap();
        assert_eq!(widths[0x41], 700.0);
        assert_eq!(widths[0x42], 800.0, "by its post name");
    }

    /// Without a Unicode `cmap`, a name reaches its glyph through the Mac OS
    /// Roman one, by the code Mac OS Roman gives it.
    #[test]
    fn without_a_unicode_cmap_names_read_by_mac_roman() {
        let program = sfnt().cmap(1, 0, 0x41, &[1, 2]);
        let encoding = dictionary! { "Differences" => vec![0x30.into(), "A".into()] };
        let widths = truetype(&program, 32, dictionary! { "Encoding" => encoding }).unwrap();
        assert_eq!(widths[0x30], 600.0);
        assert_eq!(widths[0x42], 700.0, "B, by the standard encoding");
    }

    /// A symbolic TrueType font reaches its glyphs by code: through the
    /// Windows symbol `cmap`, in the `0xF000` range too, or the Mac one.
    #[test]
    fn a_symbolic_truetype_font_reads_by_code() {
        let program = sfnt().cmap(3, 0, 0xF041, &[1, 2]);
        let widths = truetype(&program, 4, Dictionary::new()).unwrap();
        assert_eq!(widths[0x41], 600.0);
        assert_eq!(widths[0x42], 700.0);

        let program = sfnt().cmap(1, 0, 0x01, &[3]);
        let widths = truetype(&program, 4, Dictionary::new()).unwrap();
        assert_eq!(widths[0x01], 800.0);
    }

    /// A font that names `WinAnsiEncoding` reads by name even flagged
    /// symbolic (9.6.5.4), and a font whose by-name lookup fails falls back
    /// to its codes.
    #[test]
    fn a_latin_encoding_reads_by_name_whatever_the_flags() {
        let program = sfnt().cmap(3, 1, 0x41, &[1]).cmap(3, 0, 0xF042, &[2]);
        let widths =
            truetype(&program, 4, dictionary! { "Encoding" => "WinAnsiEncoding" }).unwrap();
        assert_eq!(widths[0x41], 600.0);
        assert_eq!(widths[0x42], 700.0, "by code, as no name reaches it");
    }

    /// A Type1 font with a CFF program reaches a code its encoding names no
    /// glyph for through the program's built-in encoding, and a named code by
    /// the program's glyph names, its widths scaled by the font matrix.
    #[test]
    fn a_cff_program_reads_by_name_and_built_in_encoding() {
        let program = Cff::new(&[(".notdef", 250), ("A", 500), ("B", 600), ("g.alt", 700)])
            .encoding(&[0x41, 0x42]);
        let widths = type1c(&program, Dictionary::new()).unwrap();
        assert_eq!(widths[0x41], 500.0);
        assert_eq!(widths[0x42], 600.0);
        assert_eq!(widths[0x43], 250.0, ".notdef");

        let encoding = dictionary! { "Differences" => vec![0x41.into(), "g.alt".into()] };
        let widths = type1c(&program, dictionary! { "Encoding" => encoding }).unwrap();
        assert_eq!(widths[0x41], 700.0);
        assert_eq!(widths[0x42], 600.0, "the built-in encoding's");

        let widths = type1c(&program, dictionary! { "Encoding" => "WinAnsiEncoding" }).unwrap();
        assert_eq!(widths[0x42], 600.0, "B, by name");
        assert_eq!(widths[0x43], 250.0, "C: no glyph");
    }

    /// No widths are read from a Type 1 program, a program that can't be
    /// parsed, a program no code reaches a glyph in, or a font of another
    /// kind.
    #[test]
    fn unread_programs_give_no_widths() {
        let mut doc = Document::with_version("1.7");
        let type1 = embedding(&mut doc, "Type1", FontFile::Type1, b"%!PS".to_vec(), 32);
        assert!(embedded_widths(&doc, &type1).is_none());
        let garbage = embedding(&mut doc, "TrueType", FontFile::TrueType, vec![0; 64], 32);
        assert!(embedded_widths(&doc, &garbage).is_none());
        assert!(
            truetype(&sfnt(), 32, Dictionary::new()).is_none(),
            "no cmap"
        );
        let mut type3 = embedding(&mut doc, "TrueType", FontFile::TrueType, sfnt().build(), 32);
        type3.set("Subtype", "Type3");
        assert!(embedded_widths(&doc, &type3).is_none());
        let bare = dictionary! { "Type" => "Font", "Subtype" => "TrueType" };
        assert!(embedded_widths(&doc, &bare).is_none());
        let opentype = embedding(
            &mut doc,
            "TrueType",
            FontFile::OpenType,
            sfnt().cmap(3, 1, 0x41, &[1]).build(),
            32,
        );
        assert_eq!(embedded_widths(&doc, &opentype).unwrap()[0x41], 600.0);
    }
}
