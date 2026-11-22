//! Where the reader places each glyph, against where PDFium places it.
//!
//! A document is read as the codec reads every document: rebuilt, every
//! font's widths and encoding pinned. Against PDFium reading the same
//! rebuilt document, each glyph the reader places must be drawn from where a
//! character PDFium reads as it does is drawn from, on the page.
//!
//! That alone can't catch a width misread from a font program, which the
//! rebuild would pin as misread, so PDFium reads the source document too,
//! each glyph moving the pen to the next as far as PDFium's does, where the
//! source gives widths every renderer reads alike: a program, through
//! FreeType, or the font's own. A font naming a standard face and giving no
//! widths is as wide as the renderer's substitute for the face makes it,
//! which differs by renderer and platform; that is what the rebuild pins,
//! so only its rebuilt document is compared.
//!
//! [`cases`] holds documents built for one kind of font or text state each;
//! [`corpus`], real documents.

mod cases;
mod corpus;
mod covered;

use std::collections::HashMap;
use std::ops::Range;

use elide_core::Error;
use lopdf::{Dictionary, Document, Object};
use pdfium_render::prelude::PdfiumError;

use super::pdfium::with_document;
use crate::content::text::TextBlock;
use crate::pdf::Pdf;
use crate::pdf::reader::DictionaryExt;
use crate::pdf::reader::font::font_file;
use crate::sanitize::Rebuild;

/// How far apart two positions may be, in points: PDFium sums a line's
/// advances as `f32`s. A thousandth of an em at 12 points is 0.012.
const TOLERANCE: f64 = 3e-3;

/// A point on the page.
type Point = (f64, f64);

/// Whether `a` and `b` are the same point, within [`TOLERANCE`].
fn same(a: Point, b: Point) -> bool {
    (a.0 - b.0).abs() <= TOLERANCE && (a.1 - b.1).abs() <= TOLERANCE
}

/// The move from `from` to `to`.
fn step(from: Point, to: Point) -> Point {
    (to.0 - from.0, to.1 - from.1)
}

/// A glyph the reader places, or a character PDFium reads: what it reads
/// as, and where on the page it is drawn from.
#[derive(Debug)]
struct Placed<T> {
    text: T,
    origin: Point,
}

/// `source` rebuilt, as the codec reads it, with every width a font gives
/// cut to a whole number, as PDFium reads it (it keeps a simple or composite
/// font's widths as integers, where the spec, Acrobat and pdf.js read the
/// number given). A Type3 font's widths are left whole, as PDFium scales
/// them.
fn rebuilt(source: &Pdf) -> Result<Pdf, String> {
    let rebuilt = source
        .read(Rebuild)
        .map_err(|e| format!("the rebuild refuses it: {e}"))?
        .pdf;
    let mut edit = rebuilt.edit();
    let doc = edit.doc_mut();
    let mut arrays = Vec::new();
    for (&id, object) in &doc.objects {
        let Ok(font) = object.as_dict() else {
            continue;
        };
        if font.name(doc, b"Type") != Some(b"Font") || font.name(doc, b"Subtype") == Some(b"Type3")
        {
            continue;
        }
        for key in [&b"Widths"[..], b"W", b"DW"] {
            match font.get(key) {
                Ok(Object::Reference(array)) => arrays.push((*array, None)),
                Ok(_) => arrays.push((id, Some(key.to_vec()))),
                Err(_) => {}
            }
        }
    }
    for (id, key) in arrays {
        let Ok(object) = doc.get_object_mut(id) else {
            continue;
        };
        let object = match (key, object) {
            (Some(key), Object::Dictionary(font)) => match font.get_mut(&key) {
                Ok(object) => object,
                Err(_) => continue,
            },
            (None, object) => object,
            _ => continue,
        };
        truncate(object);
    }
    edit.finish()
        .map_err(|e| format!("the rebuilt document can't be saved: {e}"))
}

/// Every real number in `object`, an array's however deep, cut to a whole
/// number.
fn truncate(object: &mut Object) {
    match object {
        Object::Real(number) => *object = Object::Integer(number.trunc() as i64),
        Object::Array(items) => items.iter_mut().for_each(truncate),
        _ => {}
    }
}

/// The glyphs the reader places on the page `block` reads.
fn placed(block: &TextBlock) -> Result<Vec<Placed<String>>, String> {
    let chars: Vec<char> = block.text.chars().collect();
    block
        .rects
        .iter()
        .map(|glyph| {
            let text = chars
                .get(glyph.chars.clone())
                .ok_or("a glyph's characters are past the page's text")?;
            Ok(Placed {
                text: text.iter().collect(),
                origin: glyph.origin,
            })
        })
        .collect()
}

/// The characters the reader reads on the page `block` reads but places
/// nowhere, as a clip hides them, but spaces, each with how many.
fn hidden(block: &TextBlock) -> HashMap<char, usize> {
    let mut shown = vec![false; block.text.chars().count()];
    for glyph in &block.rects {
        if let Some(shown) = shown.get_mut(glyph.chars.clone()) {
            shown.fill(true);
        }
    }
    let mut hidden = HashMap::new();
    for (c, _) in block
        .text
        .chars()
        .zip(shown)
        .filter(|(c, shown)| !shown && !c.is_whitespace())
    {
        *hidden.entry(c).or_default() += 1;
    }
    hidden
}

/// The characters PDFium reads from each page of `pdf`, but those it adds
/// itself (between words and lines). A hyphen it finds ending a line, which
/// it reads as `U+0002`, reads as the hyphen it is.
fn theirs(pdf: &Pdf) -> Result<Vec<Vec<Placed<char>>>, String> {
    let read = with_document(pdf, |doc| {
        let failed = |e: PdfiumError| Error::malformed(e.to_string());
        let mut pages = Vec::new();
        for page in doc.pages().iter() {
            let text = page.text().map_err(failed)?;
            let mut theirs: Vec<Placed<char>> = Vec::new();
            // PDFium reads UTF-16 code units: a character past the BMP is a
            // high surrogate and a low one, joined here.
            let mut high = None;
            for c in text.chars().iter() {
                if c.is_generated().map_err(failed)? {
                    continue;
                }
                let unit = c.unicode_value();
                if (0xD800..0xDC00).contains(&unit) {
                    high = Some(unit);
                    continue;
                }
                let value = match high.take() {
                    Some(high) if (0xDC00..0xE000).contains(&unit) => {
                        0x10000 + ((high - 0xD800) << 10) + (unit - 0xDC00)
                    }
                    _ => unit,
                };
                let (x, y) = c.origin().map_err(failed)?;
                let text = if c.is_hyphen().map_err(failed)? {
                    '-'
                } else {
                    char::from_u32(value).unwrap_or(char::REPLACEMENT_CHARACTER)
                };
                theirs.push(Placed {
                    text,
                    origin: (f64::from(x.value), f64::from(y.value)),
                });
            }
            pages.push(theirs);
        }
        Ok(pages)
    });
    read.map_err(|e| format!("PDFium can't read it: {e}"))
}

/// The simple fonts of `doc`, whose widths a renderer may take from a
/// program or from a face of its own.
fn simple_fonts(doc: &Document) -> impl Iterator<Item = &Dictionary> {
    doc.objects.values().filter_map(|object| {
        let font = object.as_dict().ok()?;
        let simple = matches!(
            font.name(doc, b"Subtype"),
            Some(b"Type1" | b"MMType1" | b"TrueType")
        );
        (simple && font.name(doc, b"Type") == Some(b"Font")).then_some(font)
    })
}

/// Whether every renderer reads the widths of `doc`'s simple fonts alike:
/// each gives its own or embeds a program. A composite font's widths are
/// its own, and a Type3 font's always.
fn widths_alike(doc: &Document) -> bool {
    simple_fonts(doc).all(|font| font.has(b"Widths") || font_file(doc, font).is_some())
}

/// Whether a simple font of `doc` gives no widths but embeds a program, so
/// the rebuild reads them from the program: what comparing its source
/// document checks. Where every font gives its own, PDFium reads the source
/// as the rebuilt document, but its text, which isn't pinned.
fn reads_programs(doc: &Document) -> bool {
    simple_fonts(doc).any(|font| !font.has(b"Widths") && font_file(doc, font).is_some())
}

/// Whether PDFium's character `at` in `theirs`, which the reader places no
/// glyph for, is no disagreement: it reads as nothing (whitespace, as PDFium
/// reads `.notdef`, or a control character, as it reads a code with no
/// text), or it advances nothing, drawn from where the next character is
/// (as the rebuild pins a code that selects no glyph 0 wide), or it is text
/// the reader reads but places nowhere (a clip hides it), of those `hidden`
/// still counts.
fn excused(theirs: &[Placed<char>], at: usize, hidden: &mut HashMap<char, usize>) -> bool {
    let c = &theirs[at];
    if let Some(count) = hidden.get_mut(&c.text).filter(|count| **count > 0) {
        *count -= 1;
        return true;
    }
    c.text.is_whitespace()
        || c.text.is_control()
        || theirs
            .get(at + 1)
            .is_some_and(|next| same(c.origin, next.origin))
}

/// Whether PDFium's characters from `at` in `theirs` read as `text`, as it
/// reads a ligature: as its letters.
fn reads(theirs: &[Placed<char>], at: usize, text: &str) -> bool {
    let len = text.chars().count();
    theirs.len() >= at + len
        && theirs[at..at + len]
            .iter()
            .zip(text.chars())
            .all(|(c, ours)| c.text == ours)
}

/// The side of a cell of the grid PDFium's characters are found in by
/// origin, wider than twice [`TOLERANCE`].
const CELL: f64 = 0.01;

/// How far from the text it copies a copy is drawn, as a producer draws
/// text twice to embolden or shadow it, in points.
const COPY: f64 = 3.0;

/// Compare the reader's glyphs on a page of a rebuilt document with
/// PDFium's characters: each glyph must be drawn from where a character
/// PDFium reads as it does is, paired whatever order PDFium reads them in
/// (it reverses right-to-left text). A glyph no such character is drawn
/// from where it is is an error, with where PDFium draws the nearest that
/// reads alike, but a space or a copy of text drawn over or beside itself,
/// which PDFium's text leaves out. A glyph drawn where a character PDFium reads
/// otherwise is, is placed alike but read unlike: that goes to `readings`.
/// So is a character PDFium reads that no glyph pairs, but one
/// [`excused`] by what it reads as, where it is, or what `hidden` counts.
fn compare_rebuilt(
    ours: &[Placed<String>],
    theirs: &[Placed<char>],
    errors: &mut Vec<String>,
    readings: &mut Vec<String>,
    mut hidden: HashMap<char, usize>,
) {
    let cell = |(x, y): Point| ((x / CELL).floor() as i64, (y / CELL).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, c) in theirs.iter().enumerate() {
        grid.entry(cell(c.origin)).or_default().push(i);
    }
    let mut paired = vec![false; theirs.len()];
    // PDFium's unpaired characters drawn from where `glyph` is.
    let there = |glyph: &Placed<String>, paired: &[bool]| -> Vec<usize> {
        let (x, y) = cell(glyph.origin);
        let mut there: Vec<usize> = (x - 1..=x + 1)
            .flat_map(|x| (y - 1..=y + 1).map(move |y| (x, y)))
            .filter_map(|key| grid.get(&key))
            .flatten()
            .copied()
            .filter(|&i| !paired[i] && same(theirs[i].origin, glyph.origin))
            .collect();
        there.sort_unstable();
        there
    };

    // Each glyph paired with characters that read alike where it is, none
    // of them paired already: a ligature's letters, each once.
    let mut unpaired = Vec::new();
    for (i, glyph) in ours.iter().enumerate() {
        let len = glyph.text.chars().count();
        let found = there(glyph, &paired)
            .into_iter()
            .find(|&at| reads(theirs, at, &glyph.text) && !paired[at..at + len].contains(&true));
        match found {
            Some(found) => paired[found..found + len].fill(true),
            None => unpaired.push(i),
        }
    }

    // A copy: more glyphs that read as it does are drawn near it than
    // characters PDFium reads so, of which there is one at least, as
    // PDFium's text leaves out text a producer draws over or beside itself
    // to embolden or shadow it. Both are found by the grid of cells as wide
    // as a copy is drawn from its text.
    let ours_near = Near::of(ours.iter().map(|glyph| glyph.origin));
    let theirs_near = Near::of(theirs.iter().map(|c| c.origin));
    let copied = |glyph: &Placed<String>, unlike: &HashMap<usize, &str>| {
        let ours = ours_near
            .around(glyph.origin)
            .filter(|&i| ours[i].text == glyph.text)
            .count();
        let theirs = theirs_near
            .around(glyph.origin)
            .filter(|at| {
                reads(theirs, *at, &glyph.text) || unlike.get(at) == Some(&glyph.text.as_str())
            })
            .count();
        theirs > 0 && ours > theirs
    };

    let mut unlike: HashMap<usize, &str> = HashMap::new();
    for i in unpaired {
        let glyph = &ours[i];
        // PDFium's text extraction drops spaces (it collapses a run, and
        // trims one ending a line); the glyphs about one place the pen it
        // moved.
        if glyph.text.chars().all(char::is_whitespace) {
            continue;
        }
        // Drawn where a character PDFium reads otherwise is: placed alike,
        // read unlike.
        if let Some(&other) = there(glyph, &paired).first() {
            paired[other] = true;
            unlike.insert(other, &glyph.text);
            let text = theirs[other].text;
            readings.push(format!("{:?} reads as {text:?} to PDFium", glyph.text));
            continue;
        }
        if copied(glyph, &unlike) {
            continue;
        }
        let first = glyph.text.chars().next();
        let nearest = theirs
            .iter()
            .filter(|c| Some(c.text) == first)
            .map(|c| c.origin)
            .min_by(|a, b| distance(*a, glyph.origin).total_cmp(&distance(*b, glyph.origin)));
        errors.push(match nearest {
            Some(nearest) => format!(
                "{:?} is drawn from {:?}, PDFium's nearest from {nearest:?}",
                glyph.text, glyph.origin
            ),
            None => format!(
                "the reader places {:?} from {:?}, PDFium nowhere",
                glyph.text, glyph.origin
            ),
        });
    }
    for (at, _) in paired.iter().enumerate().filter(|(_, paired)| !**paired) {
        if !excused(theirs, at, &mut hidden) {
            let Placed { text, origin } = theirs[at];
            errors.push(format!(
                "PDFium reads {text:?} from {origin:?}, the reader nowhere"
            ));
        }
    }
}

/// Points found by whether they are within [`COPY`] of another, by a grid
/// of cells that wide.
struct Near {
    cells: HashMap<(i64, i64), Vec<(usize, Point)>>,
}

impl Near {
    /// The grid of `points`, each by its index.
    fn of(points: impl Iterator<Item = Point>) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<(usize, Point)>> = HashMap::new();
        for (i, point) in points.enumerate() {
            cells.entry(Self::cell(point)).or_default().push((i, point));
        }
        Self { cells }
    }

    /// The cell `point` is in.
    fn cell((x, y): Point) -> (i64, i64) {
        ((x / COPY).floor() as i64, (y / COPY).floor() as i64)
    }

    /// The indices of the points within [`COPY`] of `point`.
    fn around(&self, point: Point) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = Self::cell(point);
        (x - 1..=x + 1)
            .flat_map(move |x| (y - 1..=y + 1).map(move |y| (x, y)))
            .filter_map(|key| self.cells.get(&key))
            .flatten()
            .filter(move |(_, other)| distance(*other, point) <= COPY)
            .map(|(i, _)| *i)
    }
}

/// How far apart `a` and `b` are.
fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Compare the reader's glyphs on a page of a rebuilt document with
/// PDFium's characters on the page of the source document: the move from
/// each glyph to the next must be PDFium's, where PDFium read no code
/// between them. A character PDFium reads that no glyph pairs is an error,
/// but one [`excused`], `hidden` counting what the reader places nowhere.
fn compare_source(
    ours: &[Placed<String>],
    theirs: &[Placed<char>],
    mut hidden: HashMap<char, usize>,
    errors: &mut Vec<String>,
) {
    let pairs = paired(ours, theirs, &mut hidden, errors);
    let mut steps = 0;
    for pair in pairs.windows(2) {
        let [(glyph, c, _), (next, next_c, adjacent)] = pair else {
            continue;
        };
        if !adjacent {
            continue;
        }
        steps += 1;
        let ours = step(glyph.origin, next.origin);
        let theirs = step(c.origin, next_c.origin);
        if !same(ours, theirs) {
            errors.push(format!(
                "{:?} moves the pen {ours:?}, PDFium's {theirs:?}",
                glyph.text
            ));
        }
    }
    if steps == 0 && ours.len() > 1 {
        errors.push("no move of the pen is compared".into());
    }
}

/// How many of PDFium's next characters that carry text a glyph is looked
/// for among in a source document, so that one PDFium reads none for passes
/// over none.
const LOOKAHEAD: usize = 8;

/// Pairs of the reader's glyphs and PDFium's characters in a source
/// document, each glyph with the first character that reads as it does, up
/// to [`LOOKAHEAD`] past characters that carry text, and whether PDFium read no code between it and the
/// pair before. A glyph PDFium reads no character for is an error, but a
/// space, which PDFium's text may leave out, and the glyphs after it are
/// paired still; so is a character PDFium reads that the reader places no
/// glyph for, but one [`excused`].
fn paired<'a>(
    ours: &'a [Placed<String>],
    theirs: &'a [Placed<char>],
    hidden: &mut HashMap<char, usize>,
    errors: &mut Vec<String>,
) -> Vec<(&'a Placed<String>, &'a Placed<char>, bool)> {
    let mut pairs = Vec::new();
    let mut at = 0;
    for glyph in ours {
        let mut budget = LOOKAHEAD;
        let found = (at..theirs.len())
            .take_while(|&i| {
                let within = budget > 0;
                let c = theirs[i].text;
                let carries = !(c.is_whitespace() || c.is_control());
                budget = budget.saturating_sub(usize::from(carries));
                within
            })
            .find(|&i| reads(theirs, i, &glyph.text));
        let Some(found) = found else {
            if !glyph.text.chars().all(char::is_whitespace) {
                errors.push(format!("the reader places {:?} PDFium doesn't", glyph.text));
            }
            continue;
        };
        unread(theirs, at..found, hidden, errors);
        pairs.push((glyph, &theirs[found], found == at));
        at = found + glyph.text.chars().count();
    }
    unread(theirs, at..theirs.len(), hidden, errors);
    pairs
}

/// Report each of PDFium's characters `skipped` in the source document's
/// `theirs`, which the reader places no glyph for, but those [`excused`].
fn unread(
    theirs: &[Placed<char>],
    skipped: Range<usize>,
    hidden: &mut HashMap<char, usize>,
    errors: &mut Vec<String>,
) {
    for at in skipped {
        if !excused(theirs, at, hidden) {
            let text = theirs[at].text;
            errors.push(format!("PDFium reads {text:?} the reader doesn't"));
        }
    }
}
