//! Embedded font programs (9.9): the advance widths of a simple font that
//! embeds its program and gives no `/Widths`.
//!
//! A Type 1 program (`/FontFile`), a TrueType one (`/FontFile2`), and an
//! OpenType or bare CFF one (`/FontFile3`) are read, with `read-fonts`.
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

use lopdf::{Dictionary, Document};
use read_fonts::FontRef;
use read_fonts::model::Font;
use read_fonts::model::charmap::{Encoding as CmapEncoding, EncodingKind};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::types::{GlyphId, Tag};

use super::encoding::{self, BaseEncoding, Encoding, NONSYMBOLIC, SYMBOLIC};
use crate::pdf::reader::{DictionaryExt, DocumentExt, StreamExt};

/// The keys a font descriptor holds an embedded program under (9.9).
const FONT_FILES: [&[u8]; 3] = [b"FontFile", b"FontFile2", b"FontFile3"];

/// The advance width of each code of the simple `font`, in thousandths of an
/// em, from the program it embeds; `None` when it embeds none, the program
/// can't be parsed, or no code reaches a glyph in it.
pub(crate) fn embedded_widths(doc: &Document, font: &Dictionary) -> Option<[f64; 256]> {
    let reach: Reach = match font.name(doc, b"Subtype")? {
        b"TrueType" => truetype_glyphs,
        b"Type1" | b"MMType1" => type1_glyphs,
        _ => return None,
    };
    let data = program_data(doc, font)?;
    let program = Program::parse(&data)?;
    let glyphs = reach(doc, font, &program);
    if glyphs.iter().all(Option::is_none) {
        return None;
    }
    let notdef = program.advance(GlyphId::NOTDEF).unwrap_or(0.0);
    Some(array::from_fn(|code| {
        glyphs[code]
            .and_then(|glyph| program.advance(glyph))
            .unwrap_or(notdef)
    }))
}

/// The glyph name each code of the simple `font` selects through the
/// built-in encoding of the program it embeds, `None` for a code it selects
/// no glyph by; `None` when the font embeds no program read, or one with no
/// built-in encoding (a TrueType program's codes reach glyphs by `cmap`).
pub(crate) fn built_in_names(
    doc: &Document,
    font: &Dictionary,
) -> Option<Box<[Option<String>; 256]>> {
    let data = program_data(doc, font)?;
    let program = Program::parse(&data)?;
    program.built_in(0)?;
    Some(Box::new(array::from_fn(|code| {
        let glyph = program.built_in(code as u8).flatten()?;
        Some(program.font.glyph_name(glyph)?.as_str().to_owned())
    })))
}

/// The decoded program the simple `font` embeds.
fn program_data(doc: &Document, font: &Dictionary) -> Option<Vec<u8>> {
    let descriptor = doc.resolve_dict(font.get(b"FontDescriptor").ok()?)?;
    let stream = FONT_FILES
        .iter()
        .find_map(|key| doc.resolve(descriptor.get(key).ok()?)?.as_stream().ok())?;
    stream.decoded().ok()
}

/// How each code of a font reaches its glyph in a program.
type Reach = fn(&Document, &Dictionary, &Program<'_>) -> [Option<GlyphId>; 256];

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
        None if base.is_none() => program.built_in(code as u8).flatten(),
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
    let unicode =
        program.cmap(|table| table.kind() == EncodingKind::Unicode && table.sfnt_ids().is_some());
    let mac = program.cmap(|table| table.sfnt_ids() == Some((1, 0)));
    let windows_symbol = program.cmap(|table| table.sfnt_ids() == Some((3, 0)));

    let by_name = |code: usize| {
        let name = names[code]?;
        let mapped = match (&unicode, &mac) {
            (Some(unicode), _) => program.glyph(*unicode, u32::from(encoding::unicode(name)?)),
            (None, Some(mac)) => {
                let code = BaseEncoding::MacOsRoman.code(name)?;
                program.glyph(*mac, u32::from(code))
            }
            (None, None) => None,
        };
        mapped.or_else(|| program.by_name(name))
    };
    let by_code = |code: usize| {
        let code = code as u32;
        windows_symbol
            .and_then(|table| {
                [code, 0xF000 + code, 0xF100 + code, 0xF200 + code]
                    .into_iter()
                    .find_map(|code| program.glyph(table, code))
            })
            .or_else(|| program.glyph(mac?, code))
    };
    array::from_fn(|code| {
        if symbolic {
            by_code(code).or_else(|| by_name(code))
        } else {
            by_name(code).or_else(|| by_code(code))
        }
    })
}

/// A parsed font program: a Type 1, TrueType, OpenType or bare CFF one.
struct Program<'a> {
    font: Font,
    /// The program's bytes, for an OpenType program's CFF table.
    data: &'a [u8],
}

impl<'a> Program<'a> {
    /// The program `data`, or `None` when it can't be parsed.
    fn parse(data: &'a [u8]) -> Option<Self> {
        Some(Self {
            font: Font::new(data.to_vec(), 0)?,
            data,
        })
    }

    /// The glyph named `name`, from an sfnt's `post` or CFF table, a CFF
    /// program's charset, or a Type 1 program's charstrings.
    fn by_name(&self, name: &str) -> Option<GlyphId> {
        self.font
            .glyph_by_name(name)
            .filter(|glyph| *glyph != GlyphId::NOTDEF)
    }

    /// The glyph `code` reaches through the program's built-in encoding (a
    /// Type 1 or CFF program's own, or an OpenType program's CFF table's),
    /// `Some(None)` for none; `None` when the program has no built-in
    /// encoding.
    fn built_in(&self, code: u8) -> Option<Option<GlyphId>> {
        let charmap = self.font.charmap();
        if let Some(native) = charmap
            .encodings()
            .find(|table| table.sfnt_ids().is_none() && table.kind() != EncodingKind::Unicode)
        {
            return Some(charmap.map_code(native, u32::from(code)));
        }
        let table = FontRef::new(self.data)
            .ok()?
            .table_data(Tag::new(b"CFF "))?;
        let encoding = CffFontRef::new(table.as_bytes(), 0, None)
            .ok()?
            .encoding()?;
        Some(encoding.map(code).filter(|glyph| *glyph != GlyphId::NOTDEF))
    }

    /// The first `cmap` subtable of an sfnt that `select` picks.
    fn cmap(&self, select: impl Fn(&CmapEncoding) -> bool) -> Option<CmapEncoding> {
        self.font
            .charmap()
            .encodings()
            .find(|table| table.sfnt_ids().is_some() && select(table))
    }

    /// The glyph `code` reaches through the subtable `table`, `None` for
    /// `.notdef`.
    fn glyph(&self, table: CmapEncoding, code: u32) -> Option<GlyphId> {
        self.font.charmap().map_code(table, code)
    }

    /// The advance width of `glyph`, in whole thousandths of an em, as
    /// renderers round a program's advances; `None` for a glyph the program
    /// doesn't have.
    fn advance(&self, glyph: GlyphId) -> Option<f64> {
        let em = f64::from(self.font.units_per_em());
        if glyph.to_u32() >= self.font.num_glyphs() || em <= 0.0 {
            return None;
        }
        let advance = f64::from(self.font.glyph_metrics().h_advance_exact(glyph).to_f32());
        Some((advance * 1000.0 / em).round())
    }
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::fixtures::{Cff, FontFile, Sfnt, Type1, embedding};

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

    /// A Type1 font with a Type 1 program reaches a code its encoding names
    /// no glyph for through the program's built-in encoding, and a named
    /// code by the program's glyph names.
    #[test]
    fn a_type1_program_reads_by_name_and_built_in_encoding() {
        let program = Type1::new(&[(".notdef", 250), ("A", 500), ("B", 600)])
            .encoding(&[(0x41, "A"), (0x42, "B")])
            .build();
        let widths = |extra: Dictionary| {
            let mut doc = Document::with_version("1.7");
            let mut font = embedding(&mut doc, "Type1", FontFile::Type1, program.clone(), 4);
            font.extend(&extra);
            embedded_widths(&doc, &font)
        };
        let built_in = widths(Dictionary::new()).unwrap();
        assert_eq!(built_in[0x41], 500.0);
        assert_eq!(built_in[0x42], 600.0);
        assert_eq!(built_in[0x43], 250.0, ".notdef");

        let encoding = dictionary! { "Differences" => vec![0x41.into(), "B".into()] };
        let renamed = widths(dictionary! { "Encoding" => encoding }).unwrap();
        assert_eq!(renamed[0x41], 600.0, "B, by name");
    }

    /// No widths are read from a program that can't be parsed, a program no
    /// code reaches a glyph in, or a font of another kind; a TrueType font
    /// without a `cmap` reaches its glyphs by their `post` names.
    #[test]
    fn unread_programs_give_no_widths() {
        let mut doc = Document::with_version("1.7");
        let type1 = embedding(&mut doc, "Type1", FontFile::Type1, b"%!PS".to_vec(), 32);
        assert!(embedded_widths(&doc, &type1).is_none(), "no Type 1 program");
        let garbage = embedding(&mut doc, "TrueType", FontFile::TrueType, vec![0; 64], 32);
        assert!(embedded_widths(&doc, &garbage).is_none());
        let unnamed = Sfnt::new(1000, &[(".notdef", 300), ("g1", 600)]);
        assert!(
            truetype(&unnamed, 32, Dictionary::new()).is_none(),
            "no cmap, and no glyph a code names"
        );
        let named = truetype(&sfnt(), 32, Dictionary::new()).unwrap();
        assert_eq!(named[0x41], 600.0, "A, by its post name");
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
