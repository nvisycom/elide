//! The content walker: build a page's decoded text and its glyph→bytes
//! [`OffsetMap`] by walking content operators in order, recursing through
//! `Do`-invoked Form XObjects so their text is located too.
//!
//! Walking one glyph at a time is what lets a detected character span map to the
//! exact glyph byte ranges that drew it. The walk replays the PDF text machinery:
//! it tracks the text matrix and the font-size, spacing and scaling parameters,
//! advances the pen by each glyph's width, and reads a word space wherever the
//! next glyph lands further along than its predecessor's advance accounts for
//! (see [`geometry`]). PDFs store no spaces between positioned
//! glyphs, so this is what recovers the logical text a caller runs detection over.

mod geometry;
mod nested;
mod paint;
mod run;
mod stream;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem;
use std::rc::Rc;

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Document, ObjectId};

use self::nested::{Cell, Mask, Selected};
use self::paint::Painted;
use self::run::TextRun;
use self::stream::{Frame, Graphics, ScopeFont};
use super::{FontEntry, GlyphSite, OffsetMap, OffsetRun, PenStep, StreamTarget, TextBlock};
use crate::pdf::Pdf;
use crate::pdf::reader::{DocumentExt, GlyphRect, MAX_CONTENT_BYTES, PageFrame, Scope, page_frame};

/// Deepest Form-XObject nesting the walker follows, a guard against pathological
/// or cyclic documents (the cycle guard already stops re-entry; this bounds
/// legitimately deep nesting).
pub(super) const MAX_XOBJECT_DEPTH: u8 = 12;

/// Total Form-XObject walks permitted per page. Depth and the cycle guard bound
/// how *deep* and how re-entrant the walk is, but a form reached through many
/// distinct paths is re-walked on each, so a small document can still fan out to
/// an enormous number of decodes. This caps the total; a genuine page draws far
/// fewer forms than this, so it only bites pathological fan-out.
pub(super) const MAX_XOBJECT_WALKS: u32 = 4_096;

/// The most content the forms and patterns one page draws decode to, all
/// told: a form drawn many times is decoded each time, so the walk budget
/// alone would let a few large forms decode without bound.
pub(super) const MAX_WALKED_BYTES: usize = 4 * MAX_CONTENT_BYTES;

/// Build one [`TextBlock`] per page by walking its content in operator order,
/// the same order lopdf's text extraction uses, so a character offset maps to
/// the glyph that drew it. Text drawn through a `Do`-invoked Form XObject is
/// walked in place, so it is located and redactable like page-level text.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a page's
/// content, resources, or an XObject stream cannot be read;
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a page draws text
/// with a font whose encoding cannot be decoded.
pub(super) fn text_blocks(source: &Pdf) -> Result<Vec<TextBlock>> {
    let doc = source.doc();
    let pages = doc.get_pages();
    let mut out = Vec::with_capacity(pages.len());
    for (&page, &page_id) in &pages {
        out.push(text_block_for_page(doc, page, page_id)?);
    }
    Ok(out)
}

/// Build the [`TextBlock`] for one page by walking its content (and the Form
/// XObjects it draws) in operator order, so a character offset maps to the glyph
/// that drew it.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the page's
/// content, resources, or an XObject stream cannot be read;
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if the page draws text
/// with a font whose encoding cannot be decoded.
fn text_block_for_page(doc: &Document, page: u32, page_id: ObjectId) -> Result<TextBlock> {
    let content_data = doc.page_content(page_id)?;
    let content = Content::decode(&content_data)
        .map_err(|e| Error::malformed(format!("page {page} content: {e}")))?;

    let mut text = String::new();
    let mut runs: Vec<OffsetRun> = Vec::new();
    let mut next_char = 0usize;
    let mut rects: Vec<GlyphRect> = Vec::new();
    let mut advances = BTreeMap::new();
    let mut steps: Vec<PenStep> = Vec::new();
    let mut lines = 0u64;
    let mut fonts: Vec<FontEntry> = Vec::new();
    let accum = TextAccum {
        text: &mut text,
        runs: &mut runs,
        next_char: &mut next_char,
        rects: &mut rects,
        advances: &mut advances,
        steps: &mut steps,
        lines: &mut lines,
        fonts: &mut fonts,
    };
    let mut walk = Walk::new(doc, page, accum);

    let scope = Scope::page(doc, page_id);
    walk.walk_stream(
        &content,
        Frame {
            stream: StreamTarget::PageContent,
            scope: &scope,
            depth: MAX_XOBJECT_DEPTH,
            inherited: None,
        },
    )?;
    walk.walk_selected()?;
    walk.place_tiles()?;
    let unplaced = walk.unplaced;

    // The page's visible rectangle and rotation, for normalizing glyph device
    // boxes to page fractions. A page missing its boxes falls back to a unit
    // frame, leaving geometry unnormalized rather than failing.
    let frame = page_frame(doc, page_id).unwrap_or(PageFrame::UNIT);

    Ok(TextBlock {
        page,
        page_id,
        text,
        offsets: OffsetMap::new(runs),
        rects,
        advances,
        steps,
        frame,
        fonts,
        unplaced,
    })
}

/// The growing text, glyph map, and font table a walk appends to. Borrowed so a
/// single accumulation spans the page content and every Form XObject it draws.
struct TextAccum<'a> {
    text: &'a mut String,
    runs: &'a mut Vec<OffsetRun>,
    next_char: &'a mut usize,
    rects: &'a mut Vec<GlyphRect>,
    advances: &'a mut BTreeMap<GlyphSite, Option<f64>>,
    steps: &'a mut Vec<PenStep>,
    /// The lines numbered so far, across the page and every form it draws.
    lines: &'a mut u64,
    fonts: &'a mut Vec<FontEntry>,
}

impl TextAccum<'_> {
    /// The number of a new line, unique across the page.
    fn new_line(&mut self) -> u64 {
        *self.lines += 1;
        *self.lines
    }
}

/// The state threaded through a page's content walk, unchanged as it recurses
/// into Form XObjects: the document walked, the accumulating text/glyph/font
/// output, the pen
/// position shared across the page and every form it draws (so a word gap is
/// judged consistently across a `Do` boundary), the XObjects currently on the
/// recursion stack (cycle guard), the 1-based page number for diagnostics, and the
/// remaining total-walk budget.
struct Walk<'a, 'd> {
    doc: &'d Document,
    accum: TextAccum<'a>,
    /// Where the last shown glyph left the pen, in device space, shared across
    /// every stream of the page (`None` before the first glyph).
    pen: Option<(f64, f64)>,
    active: BTreeSet<ObjectId>,
    /// The tiling patterns already selected on this page: a pattern's walk
    /// does not depend on where it is selected, so each is walked once.
    patterns: BTreeSet<ObjectId>,
    /// The tiling patterns selected and not yet walked: each is walked once
    /// the page's own text is, so its text never falls inside the page's.
    selected: Vec<Selected<'d>>,
    /// The soft mask groups set and not yet walked, each walked once the
    /// page's own text is.
    masks: Vec<Mask<'d>>,
    /// The regions tiling patterns paint on this page, each with the
    /// pattern whose cell it was painted in, if any.
    painted: Vec<(Option<ObjectId>, Painted)>,
    /// The tiling pattern whose cell is being walked, if any.
    cell: Option<ObjectId>,
    /// The tiling patterns' cells walked, their glyph boxes held back to be
    /// placed on their tiles.
    cells: BTreeMap<ObjectId, Cell<'d>>,
    page: u32,
    /// Form XObject walks still permitted on this page. The stack-based `active`
    /// guard stops re-entry (cycles) but not re-walks: a form reached through
    /// several distinct paths is walked once per path, so a shallow tree that
    /// draws each child many times per level can fan out to a huge number of
    /// stream decodes without ever recursing cyclically. This bounds the total,
    /// independent of nesting depth.
    walks_remaining: u32,
    /// The bytes of form and pattern content the page's walk may still
    /// decode.
    decoded_remaining: usize,
    /// Each font held as an object of its own, read once for the page.
    read_fonts: HashMap<ObjectId, Rc<ScopeFont>>,
    /// Whether a glyph was drawn that can't be placed: in a font whose
    /// advances can't be known, or written vertically.
    unplaced: bool,
}

impl<'a, 'd> Walk<'a, 'd> {
    /// A walk of page `page` of `doc`, appending to `accum`, with nothing
    /// walked yet and the page's whole budget left.
    fn new(doc: &'d Document, page: u32, accum: TextAccum<'a>) -> Self {
        Self {
            doc,
            accum,
            pen: None,
            active: BTreeSet::new(),
            patterns: BTreeSet::new(),
            selected: Vec::new(),
            masks: Vec::new(),
            painted: Vec::new(),
            cell: None,
            cells: BTreeMap::new(),
            page,
            walks_remaining: MAX_XOBJECT_WALKS,
            decoded_remaining: MAX_WALKED_BYTES,
            read_fonts: HashMap::new(),
            unplaced: false,
        }
    }

    /// Walk the tiling patterns and soft mask groups the page selected, once
    /// its own text is read, each apart, and what they select in turn after
    /// them.
    ///
    /// # Errors
    ///
    /// As [`walk_pattern`](Self::walk_pattern) and
    /// [`walk_soft_mask`](Self::walk_soft_mask).
    fn walk_selected(&mut self) -> Result<()> {
        loop {
            let selected = mem::take(&mut self.selected);
            let masks = mem::take(&mut self.masks);
            if selected.is_empty() && masks.is_empty() {
                return Ok(());
            }
            for pattern in selected {
                self.walk_pattern(pattern)?;
            }
            for mask in masks {
                self.walk_soft_mask(mask)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::content::Operation;
    use lopdf::{Dictionary, Object, Stream, dictionary};

    use super::*;
    use crate::fixtures::{DocBuilder, Page, encoded, shown, text, tm, to_unicode_cmap, type1};

    /// Build a one-page document whose single content stream is `ops`, drawing
    /// with a simple Helvetica font (no `/Widths`, so glyph advances are the
    /// standard face's), on a 100×200 point page. Returns the walked block.
    fn extract_block(ops: Vec<Operation>) -> super::TextBlock {
        extract_block_with_widths(ops, false)
    }

    /// As [`extract_block`], optionally giving the font a `/Widths` array (so glyph
    /// advances — and thus box widths — are known) and a descriptor with
    /// ascent/descent (so box heights are real rather than the default em box).
    fn extract_block_with_widths(ops: Vec<Operation>, widths: bool) -> super::TextBlock {
        let mut font = type1("Helvetica");
        if widths {
            // Every byte 0..=255 is 500/1000 em wide — enough to give boxes a real
            // width without modelling real Helvetica metrics.
            font.set("FirstChar", 0);
            font.set("LastChar", 255);
            font.set("Widths", vec![Object::Integer(500); 256]);
            font.set(
                "FontDescriptor",
                dictionary! {
                    "Type" => "FontDescriptor",
                    "FontName" => "Helvetica",
                    "Ascent" => 718,
                    "Descent" => -207,
                },
            );
        }
        extract_block_in(ops, font)
    }

    /// As [`extract_block`], drawing with `font`.
    fn extract_block_in(ops: Vec<Operation>, font: Dictionary) -> super::TextBlock {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(font);
        pdf.page(Page::new().font("F1", font).ops(ops).size(100, 200));
        let (doc, pages) = pdf.finish();
        text_block_for_page(&doc, 1, pages[0]).unwrap()
    }

    /// The extracted page text for `ops`.
    fn extract_text(ops: Vec<Operation>) -> String {
        extract_block(ops).text
    }

    fn show(s: &str) -> Operation {
        Operation::new("Tj", vec![Object::string_literal(s)])
    }

    /// A glyph drawn at a known position yields a page-normalized rect: its left
    /// edge at the pen's x fraction, its box in the expected page quadrant with a
    /// top-left origin.
    #[test]
    fn a_glyph_yields_a_normalized_rect() {
        // Page 100×200. Draw "Hi" at x=10, baseline y=150 (near the top in PDF's
        // y-up space), font size 20. The font carries widths so the box has a real
        // width (not the zero-advance gap-presence fallback).
        let block = extract_block_with_widths(
            vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 20.into()]),
                Operation::new("Td", vec![10.into(), 150.into()]),
                show("Hi"),
                Operation::new("ET", vec![]),
            ],
            true,
        );
        // Resolve the whole span's rects through the page geometry, as the codec
        // does.
        let geometry = block.geometry();
        let rects = geometry.rects_for(0..block.text.chars().count());
        assert_eq!(rects.len(), 1, "one line → one rect: {rects:?}");
        let r = rects[0];
        // Left edge at x = 10/100 = 0.1.
        assert!((r.x - 0.1).abs() < 1e-6, "left {r:?}");
        // Baseline y=150 of 200 is in the page's upper third; with a top-left
        // origin the box top sits above 0.3 (1 - (150+ascent)/200 < 0.25).
        assert!(r.y < 0.3 && r.y >= 0.0, "top {r:?}");
        // The box has real extent.
        assert!(r.width > 0.0 && r.height > 0.0, "extent {r:?}");
    }

    /// A region selects exactly the glyphs whose centers it holds: boxes over
    /// two words select those words' characters and nothing between them.
    #[test]
    fn a_region_selects_the_glyphs_it_covers() {
        let block = extract_block_with_widths(
            vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 10.into()]),
                Operation::new("Td", vec![10.into(), 150.into()]),
                show("Jane and Roe"),
                Operation::new("ET", vec![]),
            ],
            true,
        );
        let geometry = block.geometry();
        let regions: Vec<_> = [0..4, 9..12]
            .into_iter()
            .flat_map(|chars| geometry.rects_for(chars))
            .collect();
        let mut chars: Vec<usize> = geometry.glyphs_in(&regions).flatten().collect();
        chars.sort_unstable();
        assert_eq!(chars, [0, 1, 2, 3, 9, 10, 11]);
    }

    /// Two words positioned by a `Td` jump, with no space character between them
    /// (the Chrome/Skia "print to PDF" shape), extract with a space between.
    #[test]
    fn a_positional_td_gap_reads_as_a_space() {
        let text = extract_text(vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 720.into()]),
            show("Email"),
            // A wide horizontal jump to the next word, no space glyph drawn.
            Operation::new("Td", vec![80.into(), 0.into()]),
            show("alice@example.com"),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(text, "Email alice@example.com");
    }

    /// The operations of `text` in `font` at 12 points, each glyph placed by
    /// its own `Td`, `advances` (in thousandths of an em) apart, as printer
    /// drivers and OCR text layers draw.
    fn one_glyph_at_a_time(text: &str, advances: &[i64]) -> Vec<Operation> {
        let mut ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![10.into(), 100.into()]),
        ];
        for (glyph, advance) in text.chars().zip(advances) {
            ops.push(show(&glyph.to_string()));
            let step = *advance as f64 * 12.0 / 1000.0;
            ops.push(Operation::new("Td", vec![step.into(), 0.into()]));
        }
        ops.push(Operation::new("ET", vec![]));
        ops
    }

    /// Text in a standard font with no `/Widths`, drawn one glyph at a time,
    /// reads as one word: each glyph advances by its width in the face, so
    /// the next starts where it ends.
    #[test]
    fn standard_font_glyphs_drawn_one_at_a_time_read_as_one_word() {
        // `a@b.io` in Helvetica and Courier, their AFM widths.
        let helvetica = one_glyph_at_a_time("a@b.io", &[556, 1015, 556, 278, 222, 556]);
        assert_eq!(extract_block(helvetica).text, "a@b.io");
        let courier = one_glyph_at_a_time("a@b.io", &[600; 6]);
        let text = extract_block_in(courier, type1("ABCDEF+CourierNew")).text;
        assert_eq!(text, "a@b.io");
    }

    /// A font with no widths a reader can know advances each glyph by
    /// nothing, so text drawn one glyph at a time reads with a space at each
    /// move, while text in one run still reads whole.
    #[test]
    fn a_font_with_unknown_widths_reads_by_gaps() {
        let ops = one_glyph_at_a_time("a@b.io", &[556, 1015, 556, 278, 222, 556]);
        let text = extract_block_in(ops, type1("Unknown")).text;
        assert_eq!(text, "a @ b . i o");
        let run = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            show("a@b.io"),
            Operation::new("ET", vec![]),
        ];
        assert_eq!(extract_block_in(run, type1("Unknown")).text, "a@b.io");
    }

    /// A page that draws glyphs in a font whose advances can't be known is
    /// unplaced; one whose fonts all give them, or that selects such a font
    /// and draws nothing in it, is not.
    #[test]
    fn a_glyph_in_a_font_without_widths_leaves_its_page_unplaced() {
        assert!(extract_block_in(shown("Jane"), type1("Custom")).unplaced);
        assert!(!extract_block_in(shown("Jane"), type1("Helvetica")).unplaced);
        let selected = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("ET", vec![]),
        ];
        assert!(!extract_block_in(selected, type1("Custom")).unplaced);
    }

    /// A font a form names is read once a page and keeps one slot, however
    /// many times the form is drawn.
    #[test]
    fn a_font_drawn_through_a_form_many_times_is_read_once() {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        let form = pdf.form(
            dictionary! { "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } },
            encoded(text("F1", 10, "Hi")),
        );
        pdf.page(
            Page::new()
                .font("F1", font)
                .xobject("Fm", form)
                .content(b"/Fm Do\n".repeat(200))
                .size(100, 200),
        );
        let (doc, pages) = pdf.finish();
        let block = text_block_for_page(&doc, 1, pages[0]).unwrap();
        assert_eq!(block.fonts.len(), 1);
        assert_eq!(block.text.matches("Hi").count(), 200);
    }

    /// A composite font of `encoding` drawing code 1 as `J`.
    fn composite(encoding: &str) -> Dictionary {
        let to_unicode = Stream::new(
            Dictionary::new(),
            to_unicode_cmap(&["<0000> <FFFF>"], &["<0001> <004A>"], &[]),
        );
        dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X", "Encoding" => encoding,
            "ToUnicode" => Object::Stream(to_unicode),
            "DescendantFonts" => vec![Object::Dictionary(dictionary! {
                "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "X",
                "CIDSystemInfo" => dictionary! {
                    "Registry" => Object::string_literal("Adobe"),
                    "Ordering" => Object::string_literal("Identity"),
                    "Supplement" => 0,
                },
            })],
        }
    }

    /// Vertical writing is not laid out, so a page drawing glyphs in a font
    /// that writes vertically is unplaced, though its widths are known, and
    /// its text is not read: a code that maps to no text doesn't stop the
    /// page from being redacted from its render.
    #[test]
    fn vertical_writing_leaves_its_page_unplaced() {
        let drawn = |bytes: &[u8]| shown(bytes.to_vec());
        let vertical = extract_block_in(drawn(b"\x00\x01"), composite("Identity-V"));
        assert!(vertical.unplaced);
        assert!(vertical.text.is_empty());
        let unmapped = extract_block_in(drawn(b"\x00\x09"), composite("Identity-V"));
        assert!(unmapped.unplaced);
        assert!(!extract_block_in(drawn(b"\x00\x01"), composite("Identity-H")).unplaced);
    }

    /// The text rise raises a glyph's box: the glyph is drawn above its
    /// baseline, though the pen is not moved.
    #[test]
    fn the_text_rise_raises_glyph_boxes() {
        let top = |rise: i64| {
            let ops = vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 10.into()]),
                Operation::new("Ts", vec![rise.into()]),
                Operation::new("Td", vec![10.into(), 100.into()]),
                show("A"),
                Operation::new("ET", vec![]),
            ];
            let block = extract_block_with_widths(ops, true);
            block.geometry().rects_for(0..1)[0].y
        };
        // The page is 200 points tall, and fractions run down from its top.
        assert!((top(0) - top(20) - 0.1).abs() < 1e-9);
    }

    /// Glyphs drawn contiguously (one `Tj`) are not split by spurious spaces.
    #[test]
    fn a_single_run_is_not_broken_up() {
        let text = extract_text(vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 720.into()]),
            show("Hello"),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(text, "Hello");
    }

    /// A new line (`Td` with a vertical move, as `T*`/line breaks produce) is a
    /// word boundary: the two lines join with a single space, not run together.
    #[test]
    fn a_line_move_separates_words() {
        let text = extract_text(vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 720.into()]),
            show("first"),
            Operation::new("Td", vec![0.into(), (-16).into()]),
            show("second"),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(text, "first second");
    }

    /// Two separate text objects (each its own `BT`/`ET`), as a form generator
    /// emits one per positioned field, are separated by a space: the gap is
    /// judged across the `BT` boundary, not swallowed by the matrix reset.
    #[test]
    fn separate_text_objects_are_separated() {
        let text = extract_text(vec![
            // First field.
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            tm(72, 700),
            show("Label:"),
            Operation::new("ET", vec![]),
            // Second field, a new text object positioned well to the right.
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            tm(200, 700),
            show("value"),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(text, "Label: value");
    }

    /// A small horizontal nudge (intra-word kerning, well under the gap
    /// threshold) does not insert a space.
    #[test]
    fn a_tiny_kern_is_not_a_space() {
        let text = extract_text(vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 720.into()]),
            show("Wa"),
            // A ~1pt kern between letters of one word: below 0.2em of a 12pt font.
            Operation::new("Td", vec![1.into(), 0.into()]),
            show("ter"),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(text, "Water");
    }

    /// A `TJ` array whose kern is large enough to be a word gap yields a space,
    /// while its small intra-word kerns do not.
    #[test]
    fn a_tj_kern_gap_reads_as_a_space() {
        let text = extract_text(vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 720.into()]),
            Operation::new(
                "TJ",
                vec![Object::Array(vec![
                    Object::string_literal("To"),
                    // small intra-word kern (−40/1000 em): not a space
                    (-40).into(),
                    Object::string_literal("ok"),
                    // large word gap (−400/1000 em ≈ 4.8pt > 0.2·12pt): a space
                    (-400).into(),
                    Object::string_literal("here"),
                ])],
            ),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(text, "Took here");
    }

    /// Text drawn inside a `Do`-invoked Form XObject is extracted, and its glyph
    /// geometry uses the inherited transform (the form's `/Matrix` composed onto
    /// the page CTM) — so a translated form lands its text where the matrix puts
    /// it, not at the content's local origin.
    #[test]
    fn form_xobject_text_inherits_the_transform() {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        // The form draws "Hi" at its own origin, and its /Matrix translates it
        // to (20, 180) on the page.
        let form = pdf.form(
            dictionary! {
                "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 20.into(), 180.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            encoded(text("F1", 10, "Hi")),
        );
        // The page just invokes the form.
        pdf.page(
            Page::new()
                .xobject("Fm", form)
                .content(b"/Fm Do".to_vec())
                .size(100, 200),
        );
        let (doc, pages) = pdf.finish();

        let block = text_block_for_page(&doc, 1, pages[0]).unwrap();
        // The form's text is extracted (inheritance did not drop it).
        assert_eq!(block.text, "Hi");
        // And it is positioned by the inherited transform: the glyph box sits
        // near the form's translated origin x = 20/100 = 0.2, not the content's
        // local x = 0.
        let geometry = block.geometry();
        let rects = geometry.rects_for(0..block.text.chars().count());
        assert_eq!(rects.len(), 1, "one line: {rects:?}");
        assert!(
            (rects[0].x - 0.2).abs() < 1e-6,
            "inherited translate: {:?}",
            rects[0]
        );
    }

    /// Mirrored text (negative horizontal scale) advances the pen *backwards*
    /// along the matrix x-basis. A forward positional gap in that direction still
    /// reads as a word space, while a small intra-word `TJ` kern does not — the
    /// gap is judged along the glyph-advance direction, with a positive threshold.
    #[test]
    fn mirrored_text_spacing_follows_the_advance_direction() {
        // `Tz -100` mirrors horizontally; draw at x=90 so the mirrored advance
        // runs leftwards across the 100-wide page.
        let text = extract_text(vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tz", vec![(-100).into()]),
            tm(90, 100),
            Operation::new(
                "TJ",
                vec![Object::Array(vec![
                    Object::string_literal("To"),
                    (-40).into(), // small intra-word kern: not a space
                    Object::string_literal("ok"),
                    (-400).into(), // wide word gap: a space
                    Object::string_literal("here"),
                ])],
            ),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(text, "Took here");
    }

    /// A Type3 font whose `/FontMatrix` mirrors it advances each glyph
    /// *backwards*; a `TJ` number still moves the pen by its own sign, so a
    /// word gap in that font is a positive number. Spacing is judged along the
    /// glyph's advance, so the small adjustment joins and the wide one splits.
    #[test]
    fn a_mirrored_type3_font_spaces_along_its_advance() {
        let font = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type3",
            "FontMatrix" => vec![(-0.001).into(), 0.into(), 0.into(), 0.001.into(), 0.into(), 0.into()],
            "FontBBox" => vec![0.into(), 0.into(), 500.into(), 700.into()],
            "CharProcs" => Dictionary::new(),
            "Encoding" => dictionary! { "BaseEncoding" => "WinAnsiEncoding" },
            "FirstChar" => 0,
            "LastChar" => 255,
            "Widths" => vec![Object::Integer(500); 256],
        };
        let ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![90.into(), 100.into()]),
            Operation::new(
                "TJ",
                vec![Object::Array(vec![
                    Object::string_literal("To"),
                    40.into(), // small adjustment along the advance: not a space
                    Object::string_literal("ok"),
                    400.into(), // wide gap along the advance: a space
                    Object::string_literal("here"),
                ])],
            ),
            Operation::new("ET", vec![]),
        ];
        assert_eq!(extract_block_in(ops, font).text, "Took here");
    }

    /// A Form XObject that draws text *before its own `Tf`* uses the inherited
    /// font's advance widths, so the glyph box has a real width (not the
    /// zero-advance fallback of an uninherited font).
    #[test]
    fn form_xobject_inherits_font_widths() {
        let mut pdf = DocBuilder::with_version("1.5");
        // A font with explicit widths (500/1000 for every byte) and a descriptor.
        let descriptor = pdf.add(dictionary! {
            "Type" => "FontDescriptor", "FontName" => "Helvetica",
            "Ascent" => 718, "Descent" => -207,
        });
        let mut font = type1("Helvetica");
        font.extend(&dictionary! {
            "FirstChar" => 0, "LastChar" => 255,
            "Widths" => vec![Object::Integer(500); 256],
            "FontDescriptor" => descriptor,
        });
        let font = pdf.add(font);
        // The form draws text with no `Tf` of its own — it relies on the font the
        // caller selected before the `Do`.
        let form = pdf.form(
            dictionary! { "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } },
            b"BT 10 100 Td (Hi) Tj ET".to_vec(),
        );
        // The page selects F1 at size 20, then invokes the form.
        pdf.page(
            Page::new()
                .font("F1", font)
                .xobject("Fm", form)
                .content(b"BT /F1 20 Tf ET /Fm Do".to_vec())
                .size(100, 200),
        );
        let (doc, pages) = pdf.finish();

        let block = text_block_for_page(&doc, 1, pages[0]).unwrap();
        assert_eq!(block.text, "Hi");
        let geometry = block.geometry();
        let rects = geometry.rects_for(0..block.text.chars().count());
        assert_eq!(rects.len(), 1, "one line: {rects:?}");
        // Width > 0 proves the inherited font's widths advanced the pen; with no
        // inherited widths the advance would be zero and the box degenerate.
        assert!(
            rects[0].width > 0.0,
            "inherited widths give a real box: {:?}",
            rects[0]
        );
    }

    /// The pen is shared across the `Do` boundary in both directions: a page
    /// draws a word, invokes a form that draws a word further along, then draws a
    /// third word further still. A gap must be seen entering the form AND leaving
    /// it — all three words stay space-separated, none run together.
    #[test]
    fn pen_is_shared_across_the_form_boundary() {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        // The form draws "Beta" at x=140 (a wide gap after the page's "Alpha").
        let form = pdf.form(
            dictionary! { "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } },
            b"BT /F1 12 Tf 1 0 0 1 140 150 Tm (Beta) Tj ET".to_vec(),
        );
        // The page: "Alpha" at x=10, Do the form, then "Gamma" at x=300 (a wide
        // gap after the form's "Beta" ended around x≈160).
        pdf.page(
            Page::new()
                .font("F1", font)
                .xobject("Fm", form)
                .content(
                    b"BT /F1 12 Tf 1 0 0 1 10 150 Tm (Alpha) Tj ET /Fm Do \
                      BT /F1 12 Tf 1 0 0 1 300 150 Tm (Gamma) Tj ET"
                        .to_vec(),
                )
                .size(400, 200),
        );
        let (doc, pages) = pdf.finish();
        let page_id = pages[0];

        let block = text_block_for_page(&doc, 1, page_id).unwrap();
        // A gap entering the form (Alpha→Beta) and leaving it (Beta→Gamma) are
        // both seen: the three words are space-separated.
        assert_eq!(block.text, "Alpha Beta Gamma");
    }

    /// `q`/`Q` save and restore the *font*, not just the text state: selecting a
    /// font, saving, selecting a second font, restoring, then showing text must
    /// use the first font's widths (the font is part of the graphics state).
    #[test]
    fn q_restore_brings_back_the_selected_font() {
        let mut pdf = DocBuilder::with_version("1.5");
        let descriptor = pdf.add(dictionary! {
            "Type" => "FontDescriptor", "FontName" => "F",
            "Ascent" => 700, "Descent" => -200,
        });
        // F1 is wide (900/1000 per glyph), F2 is narrow (100/1000).
        let mut font = |name: &str, width: i64| {
            let mut font = type1(name);
            font.extend(&dictionary! {
                "FirstChar" => 0, "LastChar" => 255,
                "Widths" => vec![Object::Integer(width); 256],
                "FontDescriptor" => descriptor,
            });
            pdf.add(font)
        };
        let (wide, narrow) = (font("Wide", 900), font("Narrow", 100));
        // Select F1 (wide), q, select F2 (narrow), Q, then draw "AB".
        pdf.page(
            Page::new()
                .font("F1", wide)
                .font("F2", narrow)
                .content(b"BT 1 0 0 1 0 150 Tm /F1 10 Tf q /F2 10 Tf Q (AB) Tj ET".to_vec())
                .size(100, 200),
        );
        let (doc, pages) = pdf.finish();
        let page_id = pages[0];

        let block = text_block_for_page(&doc, 1, page_id).unwrap();
        let geometry = block.geometry();
        let rects = geometry.rects_for(0..block.text.chars().count());
        assert_eq!(rects.len(), 1, "one line: {rects:?}");
        // "AB" at F1's 900/1000 × 10pt × 2 glyphs = 18pt wide, /100-pt page =
        // 0.18. With the bug (font left as F2, 100/1000) it would be ~0.02.
        assert!(
            rects[0].width > 0.1,
            "Q restored the wide font (width {} should be ~0.18, not ~0.02)",
            rects[0].width
        );
    }

    /// A one-page document whose page draws `ops` with Helvetica as `/F1` and
    /// the XObjects `/X0`, `/X1`, … that `xobjects` adds to it, returning the
    /// document and its page id.
    fn page_drawing(
        ops: Vec<Operation>,
        xobjects: impl FnOnce(&mut Document) -> Vec<ObjectId>,
    ) -> (Document, ObjectId) {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        let mut page = Page::new().font("F1", font).ops(ops).size(100, 200);
        for (i, id) in xobjects(pdf.doc()).into_iter().enumerate() {
            page = page.xobject(&format!("X{i}"), id);
        }
        pdf.page(page);
        let (doc, pages) = pdf.finish();
        (doc, pages[0])
    }

    /// A form XObject drawing `ops`, its names resolved in `resources`.
    fn form(ops: Vec<Operation>, resources: Dictionary) -> Stream {
        Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "Resources" => resources },
            Content { operations: ops }.encode().unwrap(),
        )
    }

    #[test]
    fn text_shown_before_a_font_is_an_error() {
        let ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tj", vec![Object::string_literal("Jane")]),
            Operation::new("ET", vec![]),
        ];
        let (doc, page_id) = page_drawing(ops, |_| Vec::new());
        let err = text_block_for_page(&doc, 1, page_id).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    /// Text in a form nested past the depth limit is never skipped.
    #[test]
    fn a_form_nested_too_deep_is_an_error() {
        let (doc, page_id) = page_drawing(vec![Operation::new("Do", vec!["X0".into()])], |doc| {
            let font = doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            });
            let mut inner = doc.add_object(form(
                shown("Hello"),
                dictionary! { "Font" => dictionary! { "F1" => font } },
            ));
            for _ in 0..=MAX_XOBJECT_DEPTH {
                inner = doc.add_object(form(
                    vec![Operation::new("Do", vec!["In".into()])],
                    dictionary! { "XObject" => dictionary! { "In" => inner } },
                ));
            }
            vec![inner]
        });
        let err = text_block_for_page(&doc, 1, page_id).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ResourceLimit);
    }

    #[test]
    fn a_missing_content_stream_is_an_error() {
        let (mut doc, page_id) = page_drawing(shown("Hello"), |_| Vec::new());
        let content = doc.get_page_contents(page_id)[0];
        doc.objects.remove(&content);
        assert!(text_block_for_page(&doc, 1, page_id).is_err());
    }
}
