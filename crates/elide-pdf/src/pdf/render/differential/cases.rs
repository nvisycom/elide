//! Documents built for one kind of font or text state each.

use lopdf::content::Operation;
use lopdf::{Dictionary, Object, Stream, StringFormat, dictionary};
use read_fonts::types::Tag;
use read_fonts::{FontRef, TableProvider};

use super::{
    Placed, compare_rebuilt, compare_source, hidden, placed, reads_programs, rebuilt, theirs,
};
use crate::content::text::Text;
use crate::fixtures::{
    Cff, DocBuilder, FontFile, Page, Sfnt, Type1, embedding, to_unicode_cmap, type1,
};
use crate::pdf::Pdf;

/// A case: a document, and whether its source document is compared too, as
/// it is where a font gives no widths but embeds a program the rebuild reads
/// them from ([`reads_programs`]), unless the case says the two read it
/// unlike.
struct Case {
    name: String,
    bytes: Vec<u8>,
    source: bool,
}

impl Case {
    /// A case named `name` of the document `bytes`.
    fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            bytes,
            source: true,
        }
    }

    /// The case with only its rebuilt document compared, as PDFium reads its
    /// program unlike the reader.
    fn rebuilt_only(mut self) -> Self {
        self.source = false;
        self
    }

    /// What the reader and PDFium disagree on, each prefixed by the case and
    /// the document PDFium read.
    fn errors(&self) -> Vec<String> {
        let errors = match self.compared() {
            Ok(errors) => errors,
            Err(error) => vec![error],
        };
        errors
            .into_iter()
            .map(|e| format!("{}: {e}", self.name))
            .collect()
    }

    /// What the reader and PDFium disagree on, or why they can't be
    /// compared.
    fn compared(&self) -> Result<Vec<String>, String> {
        let source = Pdf::open(&self.bytes).map_err(|e| format!("it can't be opened: {e}"))?;
        let rebuilt = rebuilt(&source)?;
        let blocks = rebuilt
            .read(Text)
            .map_err(|e| format!("the reader refuses it: {e}"))?;
        let block = blocks.first().ok_or("the reader reads no page")?;
        if block.unplaced {
            return Err("the reader can't place its glyphs".into());
        }
        let ours = placed(block)?;
        if ours.is_empty() {
            return Err("the reader places no glyph".into());
        }
        let first =
            |pages: Vec<Vec<Placed<char>>>| pages.into_iter().next().ok_or("PDFium reads no page");
        let mut found = Vec::new();
        let mut readings = Vec::new();
        compare_rebuilt(
            &ours,
            &first(theirs(&rebuilt)?)?,
            &mut found,
            &mut readings,
            hidden(block),
        );
        found.append(&mut readings);
        let mut errors: Vec<String> = found.drain(..).map(|e| format!("(rebuilt) {e}")).collect();
        if self.source && reads_programs(source.doc()) {
            compare_source(&ours, &first(theirs(&source)?)?, hidden(block), &mut found);
            errors.extend(found.into_iter().map(|e| format!("(source) {e}")));
        }
        Ok(errors)
    }
}

/// Text showing `codes` in `/F1` at `size`, in rows of 16 from the top left
/// of a 612 by 792 page, each ending in `|`, which every encoding here gives
/// a glyph with a width, so every code is followed on its line and how far
/// it moves the pen is told.
fn rows(size: i64, codes: &[u8]) -> Vec<Operation> {
    let mut ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), size.into()]),
        Operation::new("Td", vec![20.into(), 760.into()]),
    ];
    for (i, row) in codes.chunks(16).enumerate() {
        if i > 0 {
            ops.push(Operation::new("Td", vec![0.into(), (-24).into()]));
        }
        let row = Object::String([row, b"|"].concat(), StringFormat::Hexadecimal);
        ops.push(Operation::new("Tj", vec![row]));
    }
    ops.push(Operation::new("ET", vec![]));
    ops
}

/// A one-page document, 612 by 792, of the page `page` builds.
fn document(page: impl FnOnce(&mut DocBuilder) -> Page) -> Vec<u8> {
    let mut pdf = DocBuilder::new();
    let page = page(&mut pdf).size(612, 792);
    pdf.page(page);
    pdf.bytes()
}

/// A one-page document drawing `ops` with `font` as `/F1`.
fn drawn(font: Dictionary, ops: Vec<Operation>) -> Vec<u8> {
    document(|pdf| Page::new().font("F1", pdf.add(font)).ops(ops))
}

/// A one-page document drawing every single-byte code past the control
/// codes with `font` as `/F1`.
fn every_code(font: Dictionary) -> Vec<u8> {
    drawn(font, rows(12, &(0x20..=0xFF).collect::<Vec<u8>>()))
}

/// A one-page document drawing `content` with Helvetica as `/F1`.
fn helvetica(content: &str) -> Vec<u8> {
    document(|pdf| {
        Page::new()
            .font("F1", pdf.add(type1("Helvetica")))
            .content(content)
    })
}

/// The 14 standard faces.
const FACES: [&str; 14] = [
    "Courier",
    "Courier-Bold",
    "Courier-Oblique",
    "Courier-BoldOblique",
    "Helvetica",
    "Helvetica-Bold",
    "Helvetica-Oblique",
    "Helvetica-BoldOblique",
    "Times-Roman",
    "Times-Bold",
    "Times-Italic",
    "Times-BoldItalic",
    "Symbol",
    "ZapfDingbats",
];

/// A simple font of `subtype` naming `base_font`, its descriptor flagged
/// `flags`.
fn flagged(subtype: &str, base_font: &str, flags: i64) -> Dictionary {
    dictionary! {
        "Type" => "Font", "Subtype" => subtype, "BaseFont" => base_font,
        "FontDescriptor" => dictionary! {
            "Type" => "FontDescriptor", "FontName" => base_font, "Flags" => flags,
        },
    }
}

/// Fonts naming a standard face and embedding no program: each face, by
/// its own name and by others, with each base encoding and with
/// differences, and flagged in each way that picks the encoding a code
/// reads by. Only their rebuilt documents are compared, as they embed no
/// program: a renderer sizes and selects their glyphs by its own substitute
/// for the face.
fn standard() -> Vec<Case> {
    let mut cases = Vec::new();
    for face in FACES {
        cases.push(Case::new(face, every_code(type1(face))));
    }
    for name in [
        "Arial",
        "Arial,Bold",
        "Arial-BoldItalicMT",
        "TimesNewRoman,Italic",
        "CourierNew,Bold",
        "ABCDEF+Arial",
    ] {
        cases.push(Case::new(name, every_code(type1(name))));
    }
    for encoding in ["WinAnsiEncoding", "MacRomanEncoding", "StandardEncoding"] {
        let mut font = type1("Helvetica");
        font.set("Encoding", encoding);
        cases.push(Case::new(
            format!("Helvetica, {encoding}"),
            every_code(font),
        ));
    }
    let differences = vec![
        65.into(),
        "B".into(),
        "C".into(),
        "Omega".into(),
        ".notdef".into(),
        128.into(),
        "Euro".into(),
        "florin".into(),
        250.into(),
        "a".into(),
    ];
    let mut font = type1("Helvetica");
    font.set(
        "Encoding",
        dictionary! { "BaseEncoding" => "WinAnsiEncoding", "Differences" => differences },
    );
    cases.push(Case::new("Helvetica, differences", every_code(font)));
    // An entry neither a code nor a name is passed over.
    let mut font = type1("Helvetica");
    font.set(
        "Encoding",
        dictionary! { "Differences" => vec![65.into(), Object::string_literal("B"), "C".into()] },
    );
    cases.push(Case::new(
        "Helvetica, an invalid difference",
        every_code(font),
    ));
    let mut font = type1("Arial");
    font.set("Subtype", "TrueType");
    cases.push(Case::new("TrueType Arial", every_code(font)));
    cases.push(Case::new(
        "TrueType Arial, unflagged",
        every_code(flagged("TrueType", "Arial", 0)),
    ));
    cases.push(Case::new(
        "TrueType Arial, nonsymbolic",
        every_code(flagged("TrueType", "Arial", 32)),
    ));
    cases.push(Case::new(
        "TrueType Arial, symbolic",
        every_code(flagged("TrueType", "Arial", 4)),
    ));
    cases.push(Case::new(
        "Helvetica, symbolic",
        every_code(flagged("Type1", "Helvetica", 4)),
    ));
    cases
}

/// What text state, matrices and operators move glyphs by, drawn in
/// Helvetica, so only their rebuilt documents are compared.
fn placement() -> Vec<Case> {
    const SAMPLE: &str = "(Jane Roe, jane@roe.io 555 0100)";
    vec![
        Case::new(
            "character and word spacing",
            helvetica(&format!(
                "BT /F1 12 Tf 2.5 Tc 6 Tw 20 700 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "negative spacing",
            helvetica(&format!(
                "BT /F1 12 Tf -0.5 Tc -2 Tw 20 700 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "horizontal scaling",
            helvetica(&format!(
                "BT /F1 12 Tf 150 Tz 20 700 Td {SAMPLE} Tj 60 Tz 0 -20 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "spacing, scaled",
            helvetica(&format!(
                "BT /F1 12 Tf 2 Tc 4 Tw 130 Tz 20 700 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "rise",
            helvetica("BT /F1 12 Tf 20 700 Td (Jane ) Tj 4 Ts (Roe) Tj -3 Ts (, jane) Tj ET"),
        ),
        Case::new(
            "positioning in TJ",
            helvetica(
                "BT /F1 12 Tf 20 700 Td [(J) -120 (ane) 250 (R) 80.5 (oe) -1000 (jane@roe.io)] TJ ET",
            ),
        ),
        Case::new(
            "line operators",
            helvetica(
                "BT /F1 12 Tf 14 TL 20 700 Td (Jane) Tj T* (Roe) Tj (jane) ' 3 1 (roe.io) \" ET",
            ),
        ),
        Case::new(
            "sizes",
            helvetica("BT /F1 7.5 Tf 20 700 Td (Jane Roe) Tj /F1 31 Tf (jane) Tj ET"),
        ),
        Case::new(
            "a scaled text matrix",
            helvetica(&format!("BT /F1 12 Tf 2 0 0 3 50 600 Tm {SAMPLE} Tj ET")),
        ),
        Case::new(
            "a quarter turn",
            helvetica(&format!("BT /F1 12 Tf 0 1 -1 0 300 200 Tm {SAMPLE} Tj ET")),
        ),
        Case::new(
            "a half turn",
            helvetica(&format!("BT /F1 12 Tf -1 0 0 -1 500 500 Tm {SAMPLE} Tj ET")),
        ),
        Case::new(
            "three quarter turns",
            helvetica(&format!("BT /F1 12 Tf 0 -1 1 0 300 600 Tm {SAMPLE} Tj ET")),
        ),
        Case::new(
            "a sheared text matrix",
            helvetica(&format!(
                "BT /F1 12 Tf 1 0 0.35 1 50 600 Tm {SAMPLE} Tj 5 Ts (Roe) Tj 0 Ts 0 -20 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "synthetic italics",
            helvetica(&format!(
                "BT /F1 1 Tf 12 0 2.5 12 50 600 Tm {SAMPLE} Tj 0 -1.5 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "a mirrored text matrix",
            helvetica(&format!("BT /F1 12 Tf -1 0 0 1 500 600 Tm {SAMPLE} Tj ET")),
        ),
        Case::new(
            "negative horizontal scaling",
            helvetica(&format!(
                "BT /F1 12 Tf -100 Tz 500 700 Td {SAMPLE} Tj -60 Tz 0 -20 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "a negative font size",
            helvetica(&format!(
                "BT /F1 -12 Tf 500 700 Td {SAMPLE} Tj 2 Tc 4 Tw 0 -20 Td {SAMPLE} Tj ET"
            )),
        ),
        Case::new(
            "a mirroring CTM",
            helvetica(&format!(
                "q -1 0 0 1 612 0 cm BT /F1 12 Tf 20 700 Td {SAMPLE} Tj ET Q"
            )),
        ),
        Case::new(
            "a flipping CTM",
            helvetica(&format!(
                "q 1 0 0 -1 0 792 cm BT /F1 12 Tf 1 0 0 -1 20 100 Tm {SAMPLE} Tj ET Q"
            )),
        ),
        Case::new(
            "a sheared CTM",
            helvetica(&format!(
                "q 1 0.2 -0.3 1 100 50 cm BT /F1 12 Tf 20 600 Td {SAMPLE} Tj ET Q"
            )),
        ),
        Case::new(
            "a scaling CTM",
            helvetica(&format!(
                "q 0.5 0 0 0.75 100 100 cm BT /F1 12 Tf 20 700 Td {SAMPLE} Tj ET Q"
            )),
        ),
        Case::new(
            "a turning CTM",
            helvetica(&format!(
                "q 0 1 -1 0 400 100 cm BT /F1 12 Tf 20 20 Td {SAMPLE} Tj ET Q"
            )),
        ),
        Case::new(
            "a turned page",
            document(|pdf| {
                Page::new()
                    .font("F1", pdf.add(type1("Helvetica")))
                    .content(format!("BT /F1 12 Tf 20 700 Td {SAMPLE} Tj ET"))
                    .set("Rotate", 90)
            }),
        ),
        Case::new(
            "a form",
            document(|pdf| {
                let font = pdf.add(type1("Helvetica"));
                let form = pdf.form(
                    dictionary! {
                        "BBox" => vec![0.into(), 0.into(), 400.into(), 400.into()],
                        "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.5.into(), 50.into(), 80.into()],
                        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                    },
                    format!("BT /F1 12 Tf 10 10 Td {SAMPLE} Tj ET"),
                );
                Page::new()
                    .xobject("Fm", form)
                    .content("q 1 0 0 1 20 30 cm /Fm Do Q")
            }),
        ),
    ]
}

/// Fonts that give their own widths, which every reader takes: a standard
/// face's, and a face no reader knows.
fn own_widths() -> Vec<Case> {
    let widths = |first: i64, last: i64| -> Vec<Object> {
        (first..=last)
            .map(|code| ((code * 37) % 700 + 150).into())
            .collect()
    };
    let mut helvetica = type1("Helvetica");
    helvetica.set("FirstChar", 32);
    helvetica.set("LastChar", 255);
    helvetica.set("Widths", widths(32, 255));
    let mut unknown = flagged("TrueType", "Unknown-Regular", 32);
    unknown.set("FirstChar", 40);
    unknown.set("LastChar", 250);
    unknown.set("Widths", widths(40, 250));
    unknown.set("Encoding", "WinAnsiEncoding");
    vec![
        Case::new("Helvetica, its own widths", every_code(helvetica)),
        Case::new(
            "a face no reader knows, its own widths",
            every_code(unknown),
        ),
    ]
}

/// Type3 fonts, their widths in glyph space scaled by their matrices.
fn type3() -> Vec<Case> {
    let font = |pdf: &mut DocBuilder, matrix: [f64; 6], widths: [i64; 3]| {
        let procs: Dictionary = ["a", "b", "c"]
            .iter()
            .zip(widths)
            .map(|(name, width)| {
                let proc = pdf.stream(
                    Dictionary::new(),
                    format!("{width} 0 d0 0 0 {width} {width} re f"),
                );
                (name.as_bytes().to_vec(), Object::Reference(proc))
            })
            .collect();
        dictionary! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()],
            "FontMatrix" => matrix.iter().map(|&n| Object::Real(n as f32)).collect::<Vec<_>>(),
            "CharProcs" => procs,
            "Encoding" => dictionary! {
                "Differences" => vec![97.into(), "a".into(), "b".into(), "c".into()],
            },
            "FirstChar" => 97, "LastChar" => 99,
            "Widths" => widths.iter().map(|&w| Object::Integer(w)).collect::<Vec<_>>(),
            "Resources" => Dictionary::new(),
        }
    };
    let case = |name: &str, matrix: [f64; 6], widths: [i64; 3]| {
        let bytes = document(|pdf| {
            let font = font(pdf, matrix, widths);
            Page::new()
                .font("F1", pdf.add(font))
                .content("BT /F1 12 Tf 20 700 Td (abcabc cab) Tj ET")
        });
        Case::new(name, bytes)
    };
    vec![
        case(
            "Type3, thousandths",
            [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            [500, 720, 333],
        ),
        case(
            "Type3, hundredths",
            [0.01, 0.0, 0.0, 0.01, 0.0, 0.0],
            [50, 72, 33],
        ),
        case(
            "Type3, stretched",
            [0.002, 0.0, 0.0, 0.001, 0.0, 0.0],
            [250, 360, 167],
        ),
    ]
}

/// Fonts embedding a program and giving no widths, which are read from it.
fn embedded() -> Vec<Case> {
    let glyphs: &[(&'static str, u16)] = &[
        (".notdef", 500),
        ("A", 1229),
        ("B", 1365),
        ("C", 1479),
        ("D", 1001),
        ("E", 2048),
        ("space", 569),
    ];
    let text = "BT /F1 12 Tf 20 700 Td (ABCDE EDCBA BAD) Tj ET";
    let font = |subtype: &str, file: FontFile, program: Vec<u8>, flags: i64, extra: Dictionary| {
        document(|pdf| {
            let mut font = embedding(pdf.doc(), subtype, file, program, flags);
            for (key, value) in extra {
                let value = match value {
                    Object::Stream(stream) => pdf.add(stream).into(),
                    value => value,
                };
                font.set(key, value);
            }
            Page::new().font("F1", pdf.add(font)).content(text)
        })
    };
    // One subtable of each kind, from the space to E.
    let mut mapped = vec![0; 0x26];
    mapped[0] = 6;
    mapped[0x21..].copy_from_slice(&[1, 2, 3, 4, 5]);
    let unicode = Sfnt::new(2048, glyphs).cmap(3, 1, 0x20, &mapped);
    let symbol = Sfnt::new(2048, glyphs).cmap(3, 0, 0xF020, &mapped);
    let mac = Sfnt::new(2048, glyphs).cmap(1, 0, 0x20, &mapped);
    let thousandths: Vec<(&'static str, i16)> = glyphs
        .iter()
        .map(|&(name, advance)| (name, (i32::from(advance) * 1000 / 2048) as i16))
        .collect();
    let cff = Cff::new(&thousandths).encoding(&[0x41, 0x42, 0x43, 0x44, 0x45, 0x20]);
    let type1_program = Type1::new(
        &thousandths
            .iter()
            .map(|&(name, advance)| (name, i32::from(advance)))
            .collect::<Vec<_>>(),
    )
    .encoding(&[
        (0x41, "A"),
        (0x42, "B"),
        (0x43, "C"),
        (0x44, "D"),
        (0x45, "E"),
        (0x20, "space"),
    ]);
    let win_ansi = || dictionary! { "Encoding" => "WinAnsiEncoding" };
    // A symbolic font's codes read as text only by a map of their own.
    let to_unicode = dictionary! {
        "ToUnicode" => Stream::new(
            Dictionary::new(),
            to_unicode_cmap(&["<00> <FF>"], &[], &["<20> <20> <0020>", "<41> <45> <0041>"]),
        ),
    };
    vec![
        Case::new(
            "TrueType, Unicode cmap",
            font(
                "TrueType",
                FontFile::TrueType,
                unicode.build(),
                32,
                win_ansi(),
            ),
        ),
        Case::new(
            "TrueType, symbolic cmap",
            font(
                "TrueType",
                FontFile::TrueType,
                symbol.build(),
                4,
                to_unicode,
            ),
        ),
        Case::new(
            "TrueType, Mac cmap",
            font("TrueType", FontFile::TrueType, mac.build(), 32, win_ansi()),
        ),
        Case::new(
            "OpenType",
            font(
                "TrueType",
                FontFile::OpenType,
                unicode.build(),
                32,
                win_ansi(),
            ),
        ),
        Case::new(
            "CFF, built-in encoding",
            font("Type1", FontFile::Cff, cff.build(), 4, Dictionary::new()),
        ),
        Case::new(
            "CFF, by name",
            font("Type1", FontFile::Cff, cff.build(), 32, win_ansi()),
        ),
        Case::new(
            "Type 1, built-in encoding",
            font(
                "Type1",
                FontFile::Type1,
                type1_program.build(),
                4,
                Dictionary::new(),
            ),
        ),
    ]
}

/// Composite fonts, their widths by CID.
fn composite() -> Vec<Case> {
    let font = |descendant: Dictionary| {
        let to_unicode = Stream::new(
            Dictionary::new(),
            to_unicode_cmap(&["<0000> <FFFF>"], &[], &["<0001> <001A> <0041>"]),
        );
        let mut cid = dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Unknown",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "FontDescriptor" => dictionary! {
                "Type" => "FontDescriptor", "FontName" => "Unknown", "Flags" => 32,
            },
        };
        cid.extend(&descendant);
        document(|pdf| {
            let font = dictionary! {
                "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Unknown",
                "Encoding" => "Identity-H",
                "ToUnicode" => pdf.add(to_unicode),
                "DescendantFonts" => vec![Object::Dictionary(cid)],
            };
            Page::new().font("F1", pdf.add(font)).content(
                "BT /F1 12 Tf 20 700 Td <0001000200030004000500060007000800140015001a> Tj ET",
            )
        })
    };
    let w = vec![
        1.into(),
        vec![500.into(), 610.into(), 720.into()].into(),
        4.into(),
        8.into(),
        455.into(),
        20.into(),
        vec![1000.into(), 250.into()].into(),
    ];
    vec![
        Case::new(
            "composite, widths",
            font(dictionary! { "W" => w, "DW" => 800 }),
        ),
        Case::new(
            "composite, a default width",
            font(dictionary! { "DW" => 640 }),
        ),
        Case::new("composite, no widths", font(Dictionary::new())),
    ]
}

/// The real font programs in `testdata/fonts`, from
/// `scripts/pdf/fetch-test-fonts.sh`.
const SANS: &[u8] = include_bytes!("../../../../testdata/fonts/ElideTestSans.ttf");
const MONO: &[u8] = include_bytes!("../../../../testdata/fonts/ElideTestMono.ttf");
const SANS_CFF: &[u8] = include_bytes!("../../../../testdata/fonts/ElideTestSansCff.otf");
const SANS_JP: &[u8] = include_bytes!("../../../../testdata/fonts/ElideTestSansJP.otf");
const MATH: &[u8] = include_bytes!("../../../../testdata/fonts/ElideTestMath.ttf");
const CMR10: &[u8] = include_bytes!("../../../../testdata/fonts/cmr10.pfb");

/// Fonts embedding a real program and giving no widths: TrueType programs
/// with several `cmap` subtables, one with fewer horizontal metrics than
/// glyphs and one with a format 12 subtable past the BMP, a CFF program with
/// subroutines, as OpenType and bare, and a Type 1 program from its PFB
/// segments.
fn real() -> Vec<Case> {
    let font = |subtype: &str, file: FontFile, program: &[u8], flags: i64, extra: Dictionary| {
        document(|pdf| {
            let mut font = embedding(pdf.doc(), subtype, file, program.to_vec(), flags);
            font.extend(&extra);
            Page::new()
                .font("F1", pdf.add(font))
                .ops(rows(12, &(0x20..=0xFF).collect::<Vec<u8>>()))
        })
    };
    let win_ansi = || dictionary! { "Encoding" => "WinAnsiEncoding" };
    let bare_cff = FontRef::new(SANS_CFF)
        .ok()
        .and_then(|font| font.table_data(Tag::new(b"CFF ")))
        .map(|data| data.as_bytes().to_vec())
        .expect("the OpenType font holds a CFF table");
    // The bold mathematical capitals, past the BMP, over À onwards.
    let capitals: Vec<Object> = std::iter::once(0xC0.into())
        .chain((0x1D400..0x1D41A).map(|c: u32| Object::Name(format!("u{c:X}").into_bytes())))
        .collect();
    let math = dictionary! {
        "Encoding" => dictionary! {
            "BaseEncoding" => "WinAnsiEncoding", "Differences" => capitals,
        },
    };
    vec![
        Case::new(
            "Liberation Sans",
            font("TrueType", FontFile::TrueType, SANS, 32, win_ansi()),
        ),
        Case::new(
            "Liberation Mono, short metrics",
            font("TrueType", FontFile::TrueType, MONO, 32, win_ansi()),
        ),
        // The reader reaches each capital through the format 12 subtable;
        // PDFium, reading only the format 4 one the spec names (9.6.5.4),
        // draws `.notdef`. The rebuild pins the capitals' widths.
        Case::new(
            "Noto Sans Math, past the BMP",
            font("TrueType", FontFile::TrueType, MATH, 32, math),
        )
        .rebuilt_only(),
        Case::new(
            "Source Sans, OpenType",
            font("Type1", FontFile::OpenType, SANS_CFF, 32, win_ansi()),
        ),
        Case::new(
            "Source Sans, bare CFF",
            font("Type1", FontFile::Cff, &bare_cff, 32, win_ansi()),
        ),
        Case::new(
            "Computer Modern, Type 1",
            document(|pdf| {
                let font = embedding(pdf.doc(), "Type1", FontFile::Type1, CMR10.to_vec(), 4);
                // The codes TeX's text fonts draw; 0x20 names `suppress`,
                // whose text no reader knows.
                Page::new()
                    .font("F1", pdf.add(font))
                    .ops(rows(12, &(0x21..=0x7E).collect::<Vec<u8>>()))
            }),
        ),
    ]
}

/// A glyph of a composite font: its text, its CID and its width.
type CidGlyph = (char, u16, i64);

/// The text the composite cases draw in Liberation Sans.
const LATIN: &str = "Jane Roe, jane@roe.io 555 0100";

/// The glyphs of [`LATIN`] in Liberation Sans, each CID its glyph index, and
/// its width in thousandths of an em.
fn sans_glyphs() -> Vec<CidGlyph> {
    let font = FontRef::new(SANS).expect("Liberation Sans reads");
    let cmap = font.cmap().expect("it has a cmap");
    let hmtx = font.hmtx().expect("it has an hmtx");
    let em = f64::from(font.head().expect("it has a head").units_per_em());
    LATIN
        .chars()
        .map(|char| {
            let glyph = cmap.map_codepoint(char).expect("the subset maps it");
            let advance = f64::from(hmtx.advance(glyph).expect("it has an advance"));
            let cid = u16::try_from(glyph.to_u32()).expect("a small glyph index");
            (char, cid, (advance * 1000.0 / em).round() as i64)
        })
        .collect()
}

/// The glyphs of the Japanese subset, by the CIDs of Source Han Sans's
/// `Adobe-Identity-0` collection: its proportional Latin, kana, punctuation
/// and ideographs.
const SANS_JP_GLYPHS: [CidGlyph; 35] = [
    ('山', 15891, 1000),
    ('田', 27065, 1000),
    ('太', 14080, 1000),
    ('郎', 40706, 1000),
    (' ', 1, 224),
    ('東', 20856, 1000),
    ('京', 9721, 1000),
    ('都', 40777, 1000),
    ('千', 11657, 1000),
    ('代', 9807, 1000),
    ('区', 11633, 1000),
    ('、', 1397, 1000),
    ('個', 10261, 1000),
    ('人', 9748, 1000),
    ('情', 17903, 1000),
    ('報', 13652, 1000),
    ('。', 1398, 1000),
    ('J', 43, 535),
    ('a', 66, 563),
    ('n', 79, 610),
    ('e', 70, 554),
    ('R', 51, 635),
    ('o', 80, 606),
    ('0', 17, 555),
    ('1', 18, 555),
    ('2', 19, 555),
    ('3', 20, 555),
    ('あ', 1461, 1000),
    ('い', 1463, 1000),
    ('う', 1465, 1000),
    ('え', 1467, 1000),
    ('お', 1469, 1000),
    ('カ', 1564, 1000),
    ('タ', 1584, 1000),
    ('ナ', 1595, 1000),
];

/// A document drawing `glyphs` in two lines, the second with character and
/// word spacing, by a Type0 font over `Identity-H` whose descendant of
/// `subtype` embeds `program` as `file`, with the entries `extra` adds to it,
/// and gives each glyph's width in `/W` if `widths`. Each glyph's code, the
/// CID it draws, is what `code` gives it from its place in `glyphs`; its
/// `/ToUnicode` maps each code to the glyph's text.
fn composite_drawn(
    subtype: &str,
    file: FontFile,
    program: &[u8],
    glyphs: &[CidGlyph],
    code: &dyn Fn(usize, &CidGlyph) -> u16,
    extra: &dyn Fn(&mut DocBuilder) -> Dictionary,
    widths: bool,
) -> Vec<u8> {
    let codes: Vec<u16> = glyphs.iter().enumerate().map(|(i, g)| code(i, g)).collect();
    document(|pdf| {
        let mut cid = embedding(pdf.doc(), subtype, file, program.to_vec(), 4);
        cid.set(
            "CIDSystemInfo",
            dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
        );
        cid.extend(&extra(pdf));
        if widths {
            let w: Vec<Object> = codes
                .iter()
                .zip(glyphs)
                .flat_map(|(&code, &(.., width))| {
                    [i64::from(code).into(), vec![width.into()].into()]
                })
                .collect();
            cid.set("W", w);
        }
        let chars: Vec<String> = codes
            .iter()
            .zip(glyphs)
            .map(|(code, (char, ..))| {
                let units: String = char
                    .encode_utf16(&mut [0; 2])
                    .iter()
                    .map(|unit| format!("{unit:04X}"))
                    .collect();
                format!("<{code:04X}> <{units}>")
            })
            .collect();
        let chars: Vec<&str> = chars.iter().map(String::as_str).collect();
        let to_unicode = to_unicode_cmap(&["<0000> <FFFF>"], &chars, &[]);
        let font = dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "ABCDEF+Embedded",
            "Encoding" => "Identity-H",
            "ToUnicode" => pdf.add(Stream::new(Dictionary::new(), to_unicode)),
            "DescendantFonts" => vec![Object::Dictionary(cid)],
        };
        let shown = |codes: &[u16]| {
            let bytes = codes.iter().flat_map(|code| code.to_be_bytes()).collect();
            Object::String(bytes, StringFormat::Hexadecimal)
        };
        let (first, second) = codes.split_at(codes.len() / 2);
        Page::new().font("F1", pdf.add(font)).ops(vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![20.into(), 700.into()]),
            Operation::new("Tj", vec![shown(first)]),
            Operation::new("Td", vec![0.into(), (-24).into()]),
            Operation::new("Tc", vec![1.5.into()]),
            Operation::new("Tw", vec![6.into()]),
            Operation::new("Tj", vec![shown(second)]),
            Operation::new("ET", vec![]),
        ])
    })
}

/// Composite fonts embedding a real program, with and without `/W`: a
/// CIDFontType2 with a TrueType program, its CIDs glyph indices by
/// `Identity` or through a `/CIDToGIDMap` stream, and a CIDFontType0 with a
/// CID-keyed CFF program, bare and as OpenType, whose CIDs reach glyphs
/// through its charset. Without a `/ToUnicode` their text is unknown to
/// both readers: their `Identity` ordering has no character collection, so
/// the reader refuses them, and PDFium reads each code as the character of
/// its value.
fn composite_programs() -> Vec<Case> {
    let sans = sans_glyphs();
    let by_cid = |_: usize, &(_, cid, _): &CidGlyph| cid;
    // CIDs 1 onwards, which a `/CIDToGIDMap` stream maps to the glyphs.
    let by_order = |i: usize, _: &CidGlyph| u16::try_from(i + 1).expect("few glyphs");
    let identity = |_: &mut DocBuilder| dictionary! { "CIDToGIDMap" => "Identity" };
    let mapped = |pdf: &mut DocBuilder| {
        let mut map = vec![0; 2];
        for &(_, glyph, _) in &sans {
            map.extend(glyph.to_be_bytes());
        }
        dictionary! { "CIDToGIDMap" => pdf.add(Stream::new(Dictionary::new(), map)) }
    };
    let truetype = |code: &dyn Fn(usize, &CidGlyph) -> u16,
                    extra: &dyn Fn(&mut DocBuilder) -> Dictionary,
                    widths| {
        let (subtype, file) = ("CIDFontType2", FontFile::TrueType);
        composite_drawn(subtype, file, SANS, &sans, code, extra, widths)
    };
    let bare_jp = FontRef::new(SANS_JP)
        .ok()
        .and_then(|font| font.table_data(Tag::new(b"CFF ")))
        .map(|data| data.as_bytes().to_vec())
        .expect("the OpenType font holds a CFF table");
    let none = |_: &mut DocBuilder| Dictionary::new();
    let cff = |file, program: &[u8], widths| {
        let glyphs = &SANS_JP_GLYPHS;
        composite_drawn(
            "CIDFontType0",
            file,
            program,
            glyphs,
            &by_cid,
            &none,
            widths,
        )
    };
    vec![
        Case::new("CIDFontType2, Identity", truetype(&by_cid, &identity, true)),
        Case::new(
            "CIDFontType2, a CIDToGIDMap stream",
            truetype(&by_order, &mapped, true),
        ),
        Case::new(
            "CIDFontType2, no widths",
            truetype(&by_cid, &identity, false),
        ),
        Case::new(
            "CIDFontType0, CID-keyed CFF",
            cff(FontFile::CidCff, &bare_jp, true),
        ),
        Case::new(
            "CIDFontType0, OpenType",
            cff(FontFile::OpenType, SANS_JP, true),
        ),
        Case::new(
            "CIDFontType0, no widths",
            cff(FontFile::CidCff, &bare_jp, false),
        ),
    ]
}

/// Every case.
fn cases() -> Vec<Case> {
    [
        standard(),
        placement(),
        own_widths(),
        type3(),
        embedded(),
        real(),
        composite(),
        composite_programs(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The reader places every glyph of every case where PDFium does.
#[test]
#[ignore = "requires the native PDFium library at runtime"]
fn glyphs_are_placed_as_pdfium_places_them() {
    let errors: Vec<String> = cases().iter().flat_map(Case::errors).collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
