//! Where the reader places each glyph, against where PDFium places it.
//!
//! Each case is a document drawing text in one kind of font, read as the
//! codec reads every document: rebuilt, every font's widths and encoding
//! pinned. Against PDFium reading the same rebuilt document, each glyph the
//! reader places must start where PDFium's does along the direction the
//! text runs, PDFium's origin within the glyph's box across it.
//!
//! That alone can't catch a width misread from a font program, which the
//! rebuild would pin as misread, so PDFium reads the source document too: a
//! program through FreeType, a standard face by its own metrics. Each glyph
//! must advance as far as PDFium's does, but on the glyphs where PDFium
//! departs from Adobe's metrics. Where PDFium reads a source unlike the
//! reader, on what renderers disagree on and the rebuild pins, only the
//! rebuilt document is compared.

use elide_core::Error;
use lopdf::content::Operation;
use lopdf::{Dictionary, Object, Stream, StringFormat, dictionary};
use pdfium_render::prelude::PdfiumError;

use super::pdfium::with_document;
use crate::content::text::Text;
use crate::fixtures::{
    Cff, DocBuilder, FontFile, Page, Sfnt, Type1, embedding, to_unicode_cmap, type1,
};
use crate::pdf::Pdf;
use crate::pdf::reader::DeviceRect;
use crate::sanitize::Rebuild;

/// How far apart two positions may be, in points: PDFium keeps them as
/// `f32`s.
const TOLERANCE: f64 = 1e-3;

/// The direction a case's text runs on the page.
#[derive(Debug, Clone, Copy)]
enum Axis {
    Right,
    Up,
    Left,
    Down,
}

impl Axis {
    /// How far along the axis `point` is.
    fn along(self, (x, y): (f64, f64)) -> f64 {
        match self {
            Axis::Right => x,
            Axis::Up => y,
            Axis::Left => -x,
            Axis::Down => -y,
        }
    }

    /// Where across the axis `point` is.
    fn across(self, (x, y): (f64, f64)) -> f64 {
        match self {
            Axis::Right | Axis::Left => y,
            Axis::Up | Axis::Down => x,
        }
    }

    /// Where the glyph boxed by `rect` starts along the axis, and its extent
    /// across it.
    fn of(self, rect: &DeviceRect) -> (f64, (f64, f64)) {
        match self {
            Axis::Right => (rect.min_x, (rect.min_y, rect.max_y)),
            Axis::Up => (rect.min_y, (rect.min_x, rect.max_x)),
            Axis::Left => (-rect.max_x, (rect.min_y, rect.max_y)),
            Axis::Down => (-rect.max_y, (rect.min_x, rect.max_x)),
        }
    }
}

/// A glyph the reader places: what it reads as, where it starts along the
/// axis, and its box's extent across it.
#[derive(Debug)]
struct Ours {
    text: String,
    start: f64,
    across: (f64, f64),
}

/// A character PDFium reads: what it reads as, and its origin along and
/// across the axis.
#[derive(Debug)]
struct Theirs {
    text: char,
    start: f64,
    across: f64,
}

/// The glyphs the reader places on the first page of `pdf`.
fn ours(pdf: &Pdf, axis: Axis) -> Result<Vec<Ours>, String> {
    let blocks = pdf
        .read(Text)
        .map_err(|e| format!("the reader refuses it: {e}"))?;
    let block = blocks.first().ok_or("the reader reads no page")?;
    if block.unplaced {
        return Err("the reader can't place its glyphs".into());
    }
    let chars: Vec<char> = block.text.chars().collect();
    block
        .rects
        .iter()
        .map(|glyph| {
            let (start, across) = axis.of(&glyph.rect);
            let text = chars
                .get(glyph.chars.clone())
                .ok_or("a glyph's characters are past the page's text")?;
            Ok(Ours {
                text: text.iter().collect(),
                start,
                across,
            })
        })
        .collect()
}

/// The characters PDFium reads from the first page of `pdf`, but those it
/// adds itself (between words and lines).
fn theirs(pdf: &Pdf, axis: Axis) -> Result<Vec<Theirs>, String> {
    let read = with_document(pdf, |doc| {
        let failed = |e: PdfiumError| Error::malformed(e.to_string());
        let page = doc.pages().get(0).map_err(failed)?;
        let text = page.text().map_err(failed)?;
        let mut theirs = Vec::new();
        for c in text.chars().iter() {
            if c.is_generated().map_err(failed)? {
                continue;
            }
            let (x, y) = c.origin().map_err(failed)?;
            let origin = (f64::from(x.value), f64::from(y.value));
            theirs.push(Theirs {
                text: c.unicode_char().unwrap_or(char::REPLACEMENT_CHARACTER),
                start: axis.along(origin),
                across: axis.across(origin),
            });
        }
        Ok(theirs)
    });
    read.map_err(|e| format!("PDFium can't read it: {e}"))
}

/// Whether PDFium reading `c` where the reader places no glyph is no
/// disagreement in a source document: `c` is what PDFium reads for a code
/// that selects no glyph, a space for `.notdef`, else the code itself (a
/// control character, or a Latin-1 one).
fn carries_no_text(c: char) -> bool {
    c.is_whitespace() || c.is_control() || ('\u{80}'..='\u{FF}').contains(&c)
}

/// Whether PDFium's character `at` in `theirs`, which the reader places no
/// glyph for, is no disagreement. In a source document it must read as a
/// code that selects no glyph does. The rebuild pins such a code 0 wide, so
/// in a rebuilt document it must start where the next character on its line
/// does, whatever it reads as; the last on its line must read as such a code
/// does, as how far it advances can't be told.
fn unplaced(theirs: &[Theirs], at: usize, source: bool) -> bool {
    let c = &theirs[at];
    let next = theirs
        .get(at + 1)
        .filter(|next| (next.across - c.across).abs() <= TOLERANCE);
    match next {
        Some(next) if !source => (next.start - c.start).abs() <= TOLERANCE,
        _ => carries_no_text(c.text),
    }
}

/// Pairs of the reader's glyphs and PDFium's characters, each glyph with
/// the first of the next characters that read as it does (PDFium reads a
/// ligature as its letters), and whether PDFium read no code between it and
/// the pair before. A glyph PDFium reads no character for is an error, and
/// the glyphs after it are paired still; so is a character PDFium reads
/// that the reader places no glyph for, but one [`unplaced`] allows.
fn paired<'a>(
    ours: &'a [Ours],
    theirs: &'a [Theirs],
    source: bool,
    errors: &mut Vec<String>,
) -> Vec<(&'a Ours, &'a Theirs, bool)> {
    let reads = |at: usize, glyph: &Ours| {
        let len = glyph.text.chars().count();
        let read: String = theirs[at..].iter().take(len).map(|c| c.text).collect();
        read == glyph.text
    };
    let mut pairs = Vec::new();
    let mut at = 0;
    for glyph in ours {
        let Some(found) = (at..theirs.len()).find(|&i| reads(i, glyph)) else {
            errors.push(format!("the reader places {:?} PDFium doesn't", glyph.text));
            continue;
        };
        for skipped in at..found {
            if !unplaced(theirs, skipped, source) {
                let text = theirs[skipped].text;
                errors.push(format!("PDFium reads {text:?} the reader doesn't"));
            }
        }
        pairs.push((glyph, &theirs[found], found == at));
        at = found + glyph.text.chars().count();
    }
    pairs
}

/// A case: a document, the direction its text runs, and where PDFium reads
/// its source document differently.
struct Case {
    name: String,
    bytes: Vec<u8>,
    axis: Axis,
    source: Source,
}

/// How PDFium reads a case's source document.
enum Source {
    /// Alike, but the advances of these glyphs.
    Departs(&'static [char]),
    /// Unlike: the rebuild pins the spec's reading where renderers differ.
    Unlike,
}

impl Source {
    /// Alike, but where PDFium departs from the standard faces' metrics.
    fn metrics() -> Self {
        Source::Departs(PDFIUM_METRICS)
    }
}

impl Case {
    /// A case named `name` of the document `bytes`, its text running right.
    fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            bytes,
            axis: Axis::Right,
            source: Source::Departs(&[]),
        }
    }

    /// The case with PDFium reading its source document as `source` says.
    fn source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    /// The case with its text running along `axis`.
    fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
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
        let rebuilt = source
            .read(Rebuild)
            .map_err(|e| format!("the rebuild refuses it: {e}"))?
            .pdf;
        let ours = ours(&rebuilt, self.axis)?;
        if ours.is_empty() {
            return Err("the reader places no glyph".into());
        }
        let mut errors = Vec::new();
        compare(&ours, &theirs(&rebuilt, self.axis)?, None, &mut errors);
        let mut errors: Vec<String> = errors
            .into_iter()
            .map(|e| format!("(rebuilt) {e}"))
            .collect();
        if let Source::Departs(departs) = self.source {
            let mut found = Vec::new();
            compare(
                &ours,
                &theirs(&source, self.axis)?,
                Some(departs),
                &mut found,
            );
            errors.extend(found.into_iter().map(|e| format!("(source) {e}")));
        }
        Ok(errors)
    }
}

/// Compare where the reader places the glyphs of the rebuilt document
/// with where PDFium places them: in the rebuilt document, where each
/// starts; in the source document, how far each advances, but the glyphs
/// it `departs` on.
fn compare(ours: &[Ours], theirs: &[Theirs], departs: Option<&[char]>, errors: &mut Vec<String>) {
    let pairs = paired(ours, theirs, departs.is_some(), errors);
    let mut advances = 0;
    for (i, &(glyph, c, _)) in pairs.iter().enumerate() {
        let (low, high) = glyph.across;
        if c.across < low - TOLERANCE || c.across > high + TOLERANCE {
            errors.push(format!(
                "{:?} is placed across at {}, outside {low}..{high}",
                glyph.text, c.across
            ));
        }
        let Some(departs) = departs else {
            if (glyph.start - c.start).abs() > TOLERANCE {
                errors.push(format!(
                    "{:?} starts at {}, PDFium's at {}",
                    glyph.text, glyph.start, c.start
                ));
            }
            continue;
        };
        let Some(&(next, next_c, adjacent)) = pairs.get(i + 1) else {
            continue;
        };
        if !adjacent || (next_c.across - c.across).abs() > TOLERANCE {
            continue;
        }
        advances += 1;
        let advance = next.start - glyph.start;
        let theirs = next_c.start - c.start;
        let departed = glyph.text.chars().any(|c| departs.contains(&c));
        if (advance - theirs).abs() > TOLERANCE && !departed {
            errors.push(format!(
                "{:?} advances {advance}, PDFium's {theirs}",
                glyph.text
            ));
        }
    }
    if departs.is_some() && advances == 0 {
        errors.push("no advance is compared".into());
    }
}

/// Text showing `codes` in `/F1` at `size`, in rows of 16 from the top left
/// of a 612 by 792 page.
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
        let row = Object::String(row.to_vec(), StringFormat::Hexadecimal);
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

/// The glyphs PDFium sizes unlike Adobe's metrics for the standard faces,
/// and those Helvetica lacks, which PDFium sizes as its substitute does and
/// the reader as no glyph; the rebuild pins Adobe's widths for each.
const PDFIUM_METRICS: &[char] = &[
    '¯', '±', 'µ', '·', '÷', '√', '∑', '◊', '∂', '∞', '∏', 'π', '∫', '\u{2126}', '≈', '\u{F8FF}',
];

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
/// reads by.
fn standard() -> Vec<Case> {
    let mut cases = Vec::new();
    for face in FACES {
        cases.push(Case::new(face, every_code(type1(face))).source(Source::metrics()));
    }
    for name in [
        "Arial",
        "Arial,Bold",
        "Arial-BoldItalicMT",
        "TimesNewRoman,Italic",
        "CourierNew,Bold",
        "ABCDEF+Arial",
    ] {
        cases.push(Case::new(name, every_code(type1(name))).source(Source::metrics()));
    }
    for encoding in ["WinAnsiEncoding", "MacRomanEncoding", "StandardEncoding"] {
        let mut font = type1("Helvetica");
        font.set("Encoding", encoding);
        cases.push(
            Case::new(format!("Helvetica, {encoding}"), every_code(font)).source(Source::metrics()),
        );
    }
    // PDFium reads a font of MacExpertEncoding as WinAnsiEncoding.
    let mut font = type1("Helvetica");
    font.set("Encoding", "MacExpertEncoding");
    cases.push(Case::new("Helvetica, MacExpertEncoding", every_code(font)).source(Source::Unlike));

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
    cases.push(Case::new("Helvetica, differences", every_code(font)).source(Source::metrics()));
    // An entry neither a code nor a name is passed over; PDFium reads a
    // string as a name.
    let mut font = type1("Helvetica");
    font.set(
        "Encoding",
        dictionary! { "Differences" => vec![65.into(), Object::string_literal("B"), "C".into()] },
    );
    cases.push(
        Case::new("Helvetica, an invalid difference", every_code(font)).source(Source::Unlike),
    );
    // Read by WinAnsiEncoding; PDFium reads the codes through its
    // substitute's own cmap.
    let mut font = type1("Arial");
    font.set("Subtype", "TrueType");
    cases.push(Case::new("TrueType Arial", every_code(font)).source(Source::Unlike));
    cases.push(
        Case::new(
            "TrueType Arial, unflagged",
            every_code(flagged("TrueType", "Arial", 0)),
        )
        .source(Source::Unlike),
    );
    cases.push(
        Case::new(
            "TrueType Arial, nonsymbolic",
            every_code(flagged("TrueType", "Arial", 32)),
        )
        .source(Source::metrics()),
    );
    // Read by Mac OS Roman, as the spec reads it; PDFium reads the codes
    // through its substitute's own cmap.
    cases.push(
        Case::new(
            "TrueType Arial, symbolic",
            every_code(flagged("TrueType", "Arial", 4)),
        )
        .source(Source::Unlike),
    );
    // Read by the face's built-in encoding, as the spec reads it; PDFium
    // reads WinAnsiEncoding.
    cases.push(
        Case::new(
            "Helvetica, symbolic",
            every_code(flagged("Type1", "Helvetica", 4)),
        )
        .source(Source::Unlike),
    );
    cases
}

/// What text state, matrices and operators move glyphs by, drawn in
/// Helvetica.
fn placement() -> Vec<Case> {
    const SAMPLE: &str = "(Jane Roe, jane@roe.io 555 0100)";
    // Drawn in Helvetica, read alike but where PDFium departs from its
    // metrics.
    let placed = |name: &str, bytes| Case::new(name, bytes).source(Source::metrics());
    vec![
        placed(
            "character and word spacing",
            helvetica(&format!(
                "BT /F1 12 Tf 2.5 Tc 6 Tw 20 700 Td {SAMPLE} Tj ET"
            )),
        ),
        placed(
            "negative spacing",
            helvetica(&format!(
                "BT /F1 12 Tf -0.5 Tc -2 Tw 20 700 Td {SAMPLE} Tj ET"
            )),
        ),
        placed(
            "horizontal scaling",
            helvetica(&format!(
                "BT /F1 12 Tf 150 Tz 20 700 Td {SAMPLE} Tj 60 Tz 0 -20 Td {SAMPLE} Tj ET"
            )),
        ),
        placed(
            "spacing, scaled",
            helvetica(&format!(
                "BT /F1 12 Tf 2 Tc 4 Tw 130 Tz 20 700 Td {SAMPLE} Tj ET"
            )),
        ),
        placed(
            "rise",
            helvetica("BT /F1 12 Tf 20 700 Td (Jane ) Tj 4 Ts (Roe) Tj -3 Ts (, jane) Tj ET"),
        ),
        placed(
            "positioning in TJ",
            helvetica(
                "BT /F1 12 Tf 20 700 Td [(J) -120 (ane) 250 (R) 80.5 (oe) -1000 (jane@roe.io)] TJ ET",
            ),
        ),
        placed(
            "line operators",
            helvetica(
                "BT /F1 12 Tf 14 TL 20 700 Td (Jane) Tj T* (Roe) Tj (jane) ' 3 1 (roe.io) \" ET",
            ),
        ),
        placed(
            "sizes",
            helvetica("BT /F1 7.5 Tf 20 700 Td (Jane Roe) Tj /F1 31 Tf (jane) Tj ET"),
        ),
        placed(
            "a scaled text matrix",
            helvetica(&format!("BT /F1 12 Tf 2 0 0 3 50 600 Tm {SAMPLE} Tj ET")),
        ),
        placed(
            "a quarter turn",
            helvetica(&format!("BT /F1 12 Tf 0 1 -1 0 300 200 Tm {SAMPLE} Tj ET")),
        )
        .axis(Axis::Up),
        placed(
            "a half turn",
            helvetica(&format!("BT /F1 12 Tf -1 0 0 -1 500 500 Tm {SAMPLE} Tj ET")),
        )
        .axis(Axis::Left),
        placed(
            "three quarter turns",
            helvetica(&format!("BT /F1 12 Tf 0 -1 1 0 300 600 Tm {SAMPLE} Tj ET")),
        )
        .axis(Axis::Down),
        placed(
            "a scaling CTM",
            helvetica(&format!(
                "q 0.5 0 0 0.75 100 100 cm BT /F1 12 Tf 20 700 Td {SAMPLE} Tj ET Q"
            )),
        ),
        placed(
            "a turning CTM",
            helvetica(&format!(
                "q 0 1 -1 0 400 100 cm BT /F1 12 Tf 20 20 Td {SAMPLE} Tj ET Q"
            )),
        )
        .axis(Axis::Up),
        placed(
            "a turned page",
            document(|pdf| {
                Page::new()
                    .font("F1", pdf.add(type1("Helvetica")))
                    .content(format!("BT /F1 12 Tf 20 700 Td {SAMPLE} Tj ET"))
                    .set("Rotate", 90)
            }),
        ),
        placed(
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
    helvetica.set("LastChar", 126);
    helvetica.set("Widths", widths(32, 126));
    let mut unknown = flagged("TrueType", "Unknown-Regular", 32);
    unknown.set("FirstChar", 40);
    unknown.set("LastChar", 250);
    unknown.set("Widths", widths(40, 250));
    unknown.set("Encoding", "WinAnsiEncoding");
    vec![
        Case::new("Helvetica, its own widths", every_code(helvetica)).source(Source::metrics()),
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

/// Every case.
fn cases() -> Vec<Case> {
    [
        standard(),
        placement(),
        own_widths(),
        type3(),
        embedded(),
        composite(),
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
