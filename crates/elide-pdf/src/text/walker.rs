//! The content walker: build a page's decoded text and its glyph→bytes
//! [`OffsetMap`] by walking content operators in order, recursing through
//! `Do`-invoked Form XObjects so their text is located too.
//!
//! Walking one glyph at a time is what lets a detected character span map to the
//! exact glyph byte ranges that drew it. The walk replays the PDF text machinery:
//! it tracks the text matrix and the font-size, spacing and scaling parameters,
//! advances the pen by each glyph's width, and reads a word space wherever the
//! next glyph lands further along than its predecessor's advance accounts for
//! (see [`geometry`](super::geometry)). PDFs store no spaces between positioned
//! glyphs, so this is what recovers the logical text a caller runs detection over.

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, ErrorKind, Result};
use lopdf::content::Content;
use lopdf::{Document, Encoding, Object, ObjectId};

use super::geometry::{Cursor, DeviceRect, Matrix, PageFrame, SpaceRule, TextState};
use super::glyphs::decode_glyphs;
use super::widths::{GlyphMetrics, WidthSource, object_as_f64};
use super::{Address, FontEntry, GlyphBytes, OffsetMap, OffsetRun, StreamTarget, TextBlock};
use crate::document::Store;

/// The character code of the single-byte space, the only code `Tw` word spacing
/// applies to (ISO 32000-1 §9.3.3).
const SPACE_CODE: u32 = 32;

/// Deepest Form-XObject nesting the walker follows, a guard against pathological
/// or cyclic documents (the cycle guard already stops re-entry; this bounds
/// legitimately deep nesting).
const MAX_XOBJECT_DEPTH: u8 = 12;

/// Total Form-XObject walks permitted per page. Depth and the cycle guard bound
/// how *deep* and how re-entrant the walk is, but a form reached through many
/// distinct paths is re-walked on each, so a small document can still fan out to
/// an enormous number of decodes. This caps the total; a genuine page draws far
/// fewer forms than this, so it only bites pathological fan-out.
const MAX_XOBJECT_WALKS: u32 = 4_096;

/// A page's text and glyph map under construction, driven by the text geometry.
///
/// [`show`](TextRun::show) decodes a `Tj`/`TJ` operand list, appending each
/// glyph's characters as one [`OffsetRun`] and advancing the pen by the glyph's
/// width; where a glyph lands beyond its predecessor's advance (a positional word
/// gap, or a `TJ` kern wide enough to be one) it inserts a synthetic space run
/// that no glyph drew. The same accumulation spans the page's own content and the
/// Form XObjects it draws, so a detected span is located wherever it was drawn,
/// and each glyph run records the stream its bytes live in.
struct TextRun<'a> {
    text: &'a mut String,
    runs: &'a mut Vec<OffsetRun>,
    /// Next unconsumed decoded-character offset (kept so each run's `chars`
    /// range is filled without recounting `text`).
    next_char: &'a mut usize,
    /// The text matrix/line matrix, advanced as glyphs are shown.
    cursor: &'a mut Cursor,
    /// Where the pen ended after the last shown glyph, in device space, or `None`
    /// before any glyph: the reference the next glyph's position is compared
    /// against to decide a word space. Carried across show-ops so a gap between
    /// two separate `Tj`s is seen.
    pen: &'a mut Option<(f64, f64)>,
    /// Per-glyph device-space boxes (char range → box), parallel to `runs`, for
    /// building each entity's rendered geometry. A synthetic gap records none.
    rects: &'a mut Vec<GlyphRect>,
    /// The current parametric text state (font size, spacings, scaling, CTM).
    state: TextState,
    /// The selected font's width table, for the per-glyph advance.
    widths: &'a WidthSource,
    /// The selected font's vertical metrics, for the per-glyph box height.
    metrics: GlyphMetrics,
    /// The content stream currently being walked, stamped onto every glyph so a
    /// deletion edits the right stream.
    stream: StreamTarget,
    /// The font-table index of the currently selected font, stamped onto every
    /// glyph pushed so a deletion can later reach that font's `/ToUnicode`.
    font: u16,
}

/// One glyph's device-space box and the decoded character range it drew, so a
/// detected char span can be resolved back to the page rectangles it covers.
#[derive(Debug, Clone)]
pub(crate) struct GlyphRect {
    /// The page-local decoded character range this glyph produced.
    pub(crate) chars: std::ops::Range<usize>,
    /// The glyph's box in device (user) space, bottom-left origin.
    pub(crate) rect: DeviceRect,
}

impl TextRun<'_> {
    /// Append the characters a glyph drew (its decoded text) as one run tied to
    /// the glyph's bytes.
    fn push_glyph(&mut self, text: &str, address: Address, byte_start: usize, byte_end: usize) {
        let start = *self.next_char;
        let count = text.chars().count();
        if count == 0 {
            return;
        }
        self.text.push_str(text);
        *self.next_char += count;
        self.runs.push(OffsetRun {
            chars: start..*self.next_char,
            source: Some(GlyphBytes {
                address,
                byte_start,
                byte_end,
                font: self.font,
            }),
        });
    }

    /// Append a synthetic space that no glyph drew: one char of gap with no glyph
    /// source (a redaction leaves it alone). Coalesces: never two gaps in a row,
    /// and never a leading gap.
    fn push_gap(&mut self) {
        if self.text.is_empty() || self.text.ends_with(' ') {
            return;
        }
        let start = *self.next_char;
        self.text.push(' ');
        *self.next_char += 1;
        self.runs.push(OffsetRun {
            chars: start..*self.next_char,
            source: None,
        });
    }

    /// Draw a `Tj`/`TJ` operand list, decoding glyphs, recording their bytes, and
    /// advancing the pen so inter-glyph gaps become spaces.
    ///
    /// `operand_base` is the index of `operands[0]` within the operation's full
    /// operand list, so the recorded [`Address`] names the true operand even when
    /// the caller passes a sub-slice (as `"` does, whose string is operand 2). It
    /// is 0 when the whole operand list is passed.
    fn show(
        &mut self,
        enc: &Encoding,
        operands: &[Object],
        op: usize,
        operand_base: usize,
    ) -> Result<()> {
        for (i, value) in operands.iter().enumerate() {
            let operand = operand_base + i;
            match value {
                Object::String(bytes, _) => {
                    let address = Address {
                        stream: self.stream,
                        op,
                        operand,
                        item: None,
                    };
                    self.show_string(enc, bytes, address)?;
                }
                Object::Array(arr) => {
                    // A `TJ` array interleaves strings with kerning numbers (glyph-
                    // space thousandths, negated): a positive number moves the pen
                    // *left*, so a large-negative number is a forward gap. Each
                    // kern advances the pen exactly like a glyph's width, so a gap
                    // wide enough to be a word space is caught by the same geometry
                    // rule — no special threshold for kerns.
                    for (item_idx, item) in arr.iter().enumerate() {
                        match item {
                            Object::String(bytes, _) => {
                                let address = Address {
                                    stream: self.stream,
                                    op,
                                    operand,
                                    item: Some(item_idx),
                                };
                                self.show_string(enc, bytes, address)?;
                            }
                            Object::Integer(_) | Object::Real(_) => {
                                let kern = object_as_f64(item);
                                // Displacement is −kern/1000 × font size × hscale.
                                let tx = -kern / 1000.0 * self.state.font_size * self.state.h_scale;
                                self.cursor.advance(tx);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Decode one string operand into glyphs and lay them down, advancing the pen
    /// per glyph and inserting a space wherever a glyph lands past the gap
    /// threshold from the previous one.
    ///
    /// The gap is judged in device space, so a glyph whose width is unknown still
    /// works: its advance is taken as zero, leaving the pen at the glyph's origin,
    /// so a following glyph moved there by a `Td`/`TJ` positional jump still reads
    /// as a word break while glyphs packed at the same origin do not.
    fn show_string(&mut self, enc: &Encoding, bytes: &[u8], address: Address) -> Result<()> {
        for glyph in decode_glyphs(enc, bytes)? {
            let origin = self.cursor.device_origin(self.state.ctm);
            // The gap threshold is a fraction of the font size *in device units*,
            // measured along the writing direction — so it holds under rotation /
            // shear, not just upright text. The axis scale carries one text unit
            // to the page; guard a degenerate transform so it stays positive.
            let (axis, scale) = self.cursor.writing_axis(self.state.ctm);
            let em = (self.state.font_size * self.state.h_scale * scale).max(f64::MIN_POSITIVE);
            // A gap from where the previous glyph's advance left the pen is a word
            // space; before the first glyph there is nothing to compare against.
            if let Some(pen) = *self.pen
                && SpaceRule::needs_space(pen, origin, axis, em)
            {
                self.push_gap();
            }

            let char_start = *self.next_char;
            self.push_glyph(&glyph.text, address, glyph.byte_start, glyph.byte_end);
            let char_end = *self.next_char;

            // The glyph's advance (visible width only, no Tc/Tw) and its box in
            // text space, relative to the current pen: x from 0 to the advance,
            // y from descent to ascent (em fractions × font size). An unknown
            // width gives a zero-width advance; the box still has its height.
            let w0 = self.widths.advance(glyph.code).unwrap_or(0.0) / 1000.0;
            let glyph_w = w0 * self.state.font_size * self.state.h_scale;
            let top = self.metrics.top * self.state.font_size;
            let bottom = self.metrics.bottom * self.state.font_size;

            // Record the glyph's device-space box: project its four text-space
            // corners through the text matrix + CTM (handles rotation/shear),
            // then take their axis-aligned extent. Only for a real drawn glyph.
            if char_end > char_start {
                let ctm = self.state.ctm;
                let corners = [
                    self.cursor.device_point(ctm, 0.0, bottom),
                    self.cursor.device_point(ctm, glyph_w, bottom),
                    self.cursor.device_point(ctm, glyph_w, top),
                    self.cursor.device_point(ctm, 0.0, top),
                ];
                self.rects.push(GlyphRect {
                    chars: char_start..char_end,
                    rect: DeviceRect::of_corners(corners),
                });
            }

            // Advance the pen by this glyph's displacement:
            // ((w0 × Tfs) + Tc + Tw?) × Th. Tc/Tw add spacing beyond the glyph's
            // own width, so they affect the pen but not the glyph box above.
            let word = if glyph.code == SPACE_CODE && glyph.byte_end - glyph.byte_start == 1 {
                self.state.word_spacing
            } else {
                0.0
            };
            let tx = glyph_w + (self.state.char_spacing + word) * self.state.h_scale;
            self.cursor.advance(tx);
            *self.pen = Some(self.cursor.device_origin(self.state.ctm));
        }
        Ok(())
    }
}

/// The numeric operand at `index` as `f64`, or `0.0` when absent or non-numeric
/// (a malformed operator reads as its zero, never a hard error — positioning is
/// best-effort for spacing).
fn operand_f64(operands: &[Object], index: usize) -> f64 {
    operands.get(index).map(object_as_f64).unwrap_or(0.0)
}

/// Read six numeric operands from `base` as a [`Matrix`] `[a b c d e f]`, or
/// `None` when fewer than six are present.
fn read_matrix(operands: &[Object], base: usize) -> Option<Matrix> {
    if operands.len() < base + 6 {
        return None;
    }
    Some(Matrix {
        a: operand_f64(operands, base),
        b: operand_f64(operands, base + 1),
        c: operand_f64(operands, base + 2),
        d: operand_f64(operands, base + 3),
        e: operand_f64(operands, base + 4),
        f: operand_f64(operands, base + 5),
    })
}

/// Read a page's visible rectangle and display rotation into a [`PageFrame`],
/// for normalizing glyph device boxes to page fractions.
///
/// `/CropBox` (the displayed region) is preferred, falling back to `/MediaBox`;
/// both and `/Rotate` are inheritable, so this walks up `/Parent` for a value the
/// page itself does not set. Any missing piece defaults (a unit crop, zero
/// rotation), so a malformed page yields an unnormalized-but-valid frame rather
/// than failing — geometry is best-effort and never blocks redaction.
fn page_frame(doc: &Document, page_id: ObjectId) -> PageFrame {
    let crop = inherited_rect(doc, page_id, b"CropBox")
        .or_else(|| inherited_rect(doc, page_id, b"MediaBox"));
    let rotate = inherited_int(doc, page_id, b"Rotate").unwrap_or(0);
    // Normalize rotation to 0/90/180/270 (PDF requires a multiple of 90, but be
    // defensive against junk).
    let rotate = rotate.rem_euclid(360) as u32;
    let rotate = (rotate / 90) * 90;

    match crop {
        Some([x0, y0, x1, y1]) => PageFrame {
            crop_x0: x0.min(x1),
            crop_y0: y0.min(y1),
            crop_w: (x1 - x0).abs(),
            crop_h: (y1 - y0).abs(),
            rotate,
        },
        None => PageFrame {
            crop_x0: 0.0,
            crop_y0: 0.0,
            crop_w: 1.0,
            crop_h: 1.0,
            rotate,
        },
    }
}

/// Resolve a 4-number rectangle attribute (`MediaBox`/`CropBox`) on a page,
/// walking up `/Parent` for an inherited value. `None` if absent or malformed.
fn inherited_rect(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<[f64; 4]> {
    let mut id = page_id;
    for _ in 0..MAX_PARENT_WALK {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(array) = dict.get(key).and_then(|o| o.as_array())
            && array.len() == 4
        {
            let v = |i: usize| object_as_f64(&array[i]);
            return Some([v(0), v(1), v(2), v(3)]);
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// Resolve an integer attribute (`Rotate`) on a page, walking up `/Parent`.
fn inherited_int(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<i64> {
    let mut id = page_id;
    for _ in 0..MAX_PARENT_WALK {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(value) = dict.get(key).and_then(|o| o.as_i64()) {
            return Some(value);
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// A guard on the `/Parent` inheritance walk, against a cyclic page tree.
const MAX_PARENT_WALK: u32 = 64;

/// Build one [`TextBlock`] per page by walking its content in operator order,
/// the same order lopdf's text extraction uses, so a character offset maps to
/// the glyph that drew it. Text drawn through a `Do`-invoked Form XObject is
/// walked in place, so it is located and redactable like page-level text.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if a page's
/// content, resources, or an XObject stream cannot be read;
/// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a page draws text
/// with a font whose encoding cannot be decoded.
pub(crate) fn text_blocks(store: &Store) -> Result<Vec<TextBlock>> {
    let doc = store.doc();
    let max_page_bytes = store.max_page_bytes().get();
    let pages = doc.get_pages();
    let mut out = Vec::with_capacity(pages.len());
    for (&page, &page_id) in &pages {
        out.push(text_block_for_page(doc, page, page_id, max_page_bytes)?);
    }
    Ok(out)
}

/// Build the [`TextBlock`] for one page by walking its content (and the Form
/// XObjects it draws) in operator order, so a character offset maps to the glyph
/// that drew it.
///
/// The per-page primitive [`text_blocks`] loops over. Extraction calls it too,
/// classifying a page it cannot decode as an issue rather than failing the whole
/// document, so its error policy differs from the redactor's fail-closed one.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the page's
/// content, resources, or an XObject stream cannot be read;
/// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if the page draws text
/// with a font whose encoding cannot be decoded.
pub(crate) fn text_block_for_page(
    doc: &Document,
    page: u32,
    page_id: ObjectId,
    max_page_bytes: usize,
) -> Result<TextBlock> {
    let content_data = doc.get_page_content(page_id);
    // Guard against a decompression bomb: refuse a page whose (already
    // decompressed) content stream exceeds the bound rather than walking it.
    if content_data.len() > max_page_bytes {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            format!(
                "page {page} content is {} bytes, over the {max_page_bytes}-byte limit",
                content_data.len()
            ),
        ));
    }
    let content = Content::decode(&content_data).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("page {page} content: {e}"),
        )
    })?;

    // The page's own resources, the scope its `Tf`/`Do` names resolve in.
    let (resources, resource_ids) = doc.get_page_resources(page_id).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("page {page} resources: {e}"),
        )
    })?;

    let mut text = String::new();
    let mut runs: Vec<OffsetRun> = Vec::new();
    let mut next_char = 0usize;
    let mut rects: Vec<GlyphRect> = Vec::new();
    let mut fonts: Vec<FontEntry> = Vec::new();
    let mut ctx = Walk {
        accum: TextAccum {
            text: &mut text,
            runs: &mut runs,
            next_char: &mut next_char,
            rects: &mut rects,
            fonts: &mut fonts,
        },
        active: BTreeSet::new(),
        page,
        max_page_bytes,
        walks_remaining: MAX_XOBJECT_WALKS,
    };

    walk_stream(
        doc,
        &content,
        StreamTarget::PageContent,
        &Scope {
            inline: resources,
            referenced: &resource_ids,
        },
        MAX_XOBJECT_DEPTH,
        None,
        &mut ctx,
    )?;

    // The page's visible rectangle and rotation, for normalizing glyph device
    // boxes to page fractions. Read once per page (best-effort: a page missing
    // its boxes falls back to a unit frame, leaving geometry unnormalized rather
    // than failing — geometry never blocks extraction/redaction).
    let frame = page_frame(doc, page_id);

    Ok(TextBlock {
        page,
        page_id,
        text,
        offsets: OffsetMap::new(runs),
        rects,
        frame,
        fonts,
    })
}

/// Walk one content stream in operator order, appending its text and glyph map
/// to `accum`, and recursing into each Form XObject drawn with `Do`.
///
/// `scope` is the resource dictionary a `Tf`/`Do` resolves names in. `stream` is
/// stamped onto every glyph so a deletion edits the correct stream. `active`
/// holds the XObjects currently on the recursion stack (cycle guard); `depth`
/// bounds nesting. Text under an undecodable font fails closed, as at page level.
///
/// The caller's graphics state passed into a `Do`-invoked Form XObject: the
/// font it had selected, its parametric text state (with the form's `/Matrix`
/// already composed onto the CTM), and the pen position, so the form's text is
/// positioned in the page's coordinate space and a word gap across the boundary
/// is still judged.
struct Inherited<'a> {
    /// The caller's selected font as (table slot, resolved encoding), or `None`
    /// when the caller had selected no font. The encoding is `&Some` decodable /
    /// `&Some(None)` selected-but-undecodable.
    font: Option<(u16, &'a Option<Encoding<'a>>)>,
    /// The caller's text state, CTM composed with the form's `/Matrix`.
    state: TextState,
    /// Where the caller's last glyph left the pen, for the gap check.
    pen: Option<(f64, f64)>,
}

/// `inherited` is the caller's graphics state at the `Do` that invoked this
/// stream: a Form XObject inherits the graphics state, so text it draws before
/// its own operators uses the caller's font *and* transform. It carries the
/// font-table slot + resolved encoding and the caller's [`TextState`] (whose CTM
/// already has the form's `/Matrix` composed in). `None` at page level, where
/// the state starts fresh.
fn walk_stream(
    doc: &Document,
    content: &Content,
    stream: StreamTarget,
    scope: &Scope<'_>,
    depth: u8,
    inherited: Option<Inherited<'_>>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    // Resolve this scope's fonts to encodings, width tables, and a `FontEntry`
    // per font in the shared table. A local name->slot map points the run at the
    // right entry on each `Tf`, and name->encoding / name->widths carry what a
    // `Tf` then needs to decode and position that font's glyphs.
    let font_dicts = scope_fonts(doc, scope);
    let mut font_slot: BTreeMap<Vec<u8>, u16> = BTreeMap::new();
    let mut encodings: BTreeMap<Vec<u8>, Option<Encoding>> = BTreeMap::new();
    let mut widths: BTreeMap<Vec<u8>, WidthSource> = BTreeMap::new();
    let mut metrics: BTreeMap<Vec<u8>, GlyphMetrics> = BTreeMap::new();
    for (name, font) in &font_dicts {
        let to_unicode = font
            .get(b"ToUnicode")
            .ok()
            .and_then(|o| o.as_reference().ok());
        // The font-table index is a `u16` a glyph records to route its code to
        // the right `/ToUnicode` CMap at scrub time. Every stream walk (including
        // each re-walk allowed by the walk budget) appends its scope's fonts, so
        // the table can grow; if it would exceed the `u16` range the slot would
        // truncate and misroute the scrub, leaving deleted text recoverable.
        // Fail closed instead, as everywhere else a glyph might slip the scrub.
        let Ok(slot) = u16::try_from(ctx.accum.fonts.len()) else {
            return Err(Error::new(
                ErrorKind::Redaction,
                format!(
                    "page {} references more than {} fonts across its content \
                     streams; its text cannot be safely redacted",
                    ctx.page,
                    u16::MAX,
                ),
            ));
        };
        ctx.accum.fonts.push(FontEntry { to_unicode });
        font_slot.insert(name.clone(), slot);
        encodings.insert(name.clone(), font.get_font_encoding(doc).ok());
        widths.insert(name.clone(), WidthSource::from_font(doc, font));
        metrics.insert(name.clone(), GlyphMetrics::from_font(doc, font));
    }

    // The current font's resolved encoding: `None` before any `Tf` (unless a
    // font is inherited from the caller's graphics state, below), or `Some(None)`
    // when the selected font could not be decoded or names a font absent from
    // this scope's resources.
    const UNRESOLVED: &Option<Encoding> = &None;
    const NO_WIDTHS: &WidthSource = &WidthSource::Unknown;
    // A Form XObject inherits the caller's font, so text it draws before its own
    // `Tf` uses that font; at page level there is nothing to inherit.
    let (mut current, mut font_slot_current): (Option<&Option<Encoding>>, u16) =
        match inherited.as_ref().and_then(|inh| inh.font) {
            Some((slot, enc)) => (Some(enc), slot),
            None => (None, 0),
        };
    // The selected font's width table, selected together with the encoding on
    // `Tf`. An inherited font keeps `Unknown` until the form's own `Tf`: widths
    // are best-effort and only affect spacing.
    let mut width_current: &WidthSource = NO_WIDTHS;
    // The selected font's vertical metrics, likewise selected on `Tf`; the
    // default em box stands in before any `Tf` (metrics only size the glyph box).
    let mut metrics_current = GlyphMetrics::default();

    // The text geometry: the matrix cursor (reset at `BT`), the parametric text
    // state and its `q`/`Q` save stack, and where the last glyph left the pen
    // (kept across show-ops so a gap between two `Tj`s is seen). A Form XObject
    // inherits the caller's text state (its CTM already carries the form's
    // `/Matrix`) and pen, so text inside the form is positioned in the page's
    // coordinate space and a gap across the `Do` boundary is still seen.
    let mut cursor = Cursor::new();
    let mut state = inherited
        .as_ref()
        .map_or_else(TextState::default, |inh| inh.state);
    let mut gs_stack: Vec<TextState> = Vec::new();
    let mut pen: Option<(f64, f64)> = inherited.as_ref().and_then(|inh| inh.pen);

    for (op_idx, op) in content.operations.iter().enumerate() {
        match op.operator.as_str() {
            // Graphics-state save/restore: the text parameters and CTM nest; the
            // text matrix is not graphics state (it resets at `BT`), so it is not
            // saved. A `Q` with an empty stack is a malformed balance we ignore.
            "q" => gs_stack.push(state),
            "Q" => {
                if let Some(saved) = gs_stack.pop() {
                    state = saved;
                }
            }
            "cm" => {
                if let Some(m) = read_matrix(&op.operands, 0) {
                    state.ctm = m.then(state.ctm);
                }
            }

            // Text object: the text and line matrices reset to the identity
            // (ISO 32000-1 §9.4.1). The pen is kept across the boundary: it is a
            // device-space position, unaffected by the matrix reset, so a gap
            // between one text object's last glyph and the next object's first is
            // judged by the same geometry rule. Many producers emit one `BT`/`ET`
            // per positioned field (this form has 376), so nulling the pen here
            // would run every field into the next with no separator.
            "BT" => cursor = Cursor::new(),
            "ET" => {}

            // Text state parameters.
            "Tc" => state.char_spacing = operand_f64(&op.operands, 0),
            "Tw" => state.word_spacing = operand_f64(&op.operands, 0),
            "Tz" => state.h_scale = operand_f64(&op.operands, 0) / 100.0,
            "TL" => state.leading = operand_f64(&op.operands, 0),
            "Ts" => state.rise = operand_f64(&op.operands, 0),
            "Tf" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    current = Some(encodings.get(name).unwrap_or(UNRESOLVED));
                    width_current = widths.get(name.as_slice()).unwrap_or(NO_WIDTHS);
                    metrics_current = metrics.get(name.as_slice()).copied().unwrap_or_default();
                    if let Some(&slot) = font_slot.get(name.as_slice()) {
                        font_slot_current = slot;
                    }
                }
                state.font_size = operand_f64(&op.operands, 1);
            }

            // Text positioning: move the line/text matrix.
            "Td" => {
                cursor.next_line_offset(operand_f64(&op.operands, 0), operand_f64(&op.operands, 1))
            }
            "TD" => {
                // `TD tx ty` also sets leading to `-ty`.
                let ty = operand_f64(&op.operands, 1);
                state.leading = -ty;
                cursor.next_line_offset(operand_f64(&op.operands, 0), ty);
            }
            "Tm" => {
                if let Some(m) = read_matrix(&op.operands, 0) {
                    cursor.set_matrix(m);
                }
            }
            "T*" => cursor.next_line_offset(0.0, -state.leading),

            // Text showing. `'` is an implicit `T*` then show; `"` additionally
            // sets word and character spacing from its first two operands, then
            // shows its third.
            "Tj" | "TJ" | "'" | "\"" => {
                if op.operator == "'" {
                    cursor.next_line_offset(0.0, -state.leading);
                } else if op.operator == "\"" {
                    state.word_spacing = operand_f64(&op.operands, 0);
                    state.char_spacing = operand_f64(&op.operands, 1);
                    cursor.next_line_offset(0.0, -state.leading);
                }
                match current {
                    Some(Some(enc)) => {
                        let (operands, base): (&[Object], usize) = if op.operator == "\"" {
                            match op.operands.get(2) {
                                Some(s) => (std::slice::from_ref(s), 2),
                                None => (&[], 0),
                            }
                        } else {
                            (&op.operands, 0)
                        };
                        // A short-lived run so its borrow of `accum` ends before
                        // the `Do` arm needs `accum` again.
                        let mut run = TextRun {
                            text: ctx.accum.text,
                            runs: ctx.accum.runs,
                            next_char: ctx.accum.next_char,
                            rects: ctx.accum.rects,
                            cursor: &mut cursor,
                            pen: &mut pen,
                            state,
                            widths: width_current,
                            metrics: metrics_current,
                            stream,
                            font: font_slot_current,
                        };
                        run.show(enc, operands, op_idx, base)?;
                    }
                    Some(None) => {
                        return Err(Error::new(
                            ErrorKind::Redaction,
                            format!(
                                "page {} draws text with a font whose encoding \
                                 could not be decoded or was not found in the page \
                                 resources; its text cannot be redacted",
                                ctx.page
                            ),
                        ));
                    }
                    None => {}
                }
            }

            // A `Do` may draw a Form XObject, whose content draws text in its own
            // stream and resources. Recurse so that text is located and
            // redactable too.
            "Do" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    // Pass the current graphics state down: the form inherits the
                    // selected font, the text state (its CTM, onto which
                    // `walk_xobject` composes the form's `/Matrix`), and the pen
                    // (so a gap across the `Do` boundary is seen).
                    let inherited = Inherited {
                        font: current.map(|enc| (font_slot_current, enc)),
                        state,
                        pen,
                    };
                    walk_xobject(doc, name, scope, depth, inherited, ctx)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve and recurse into the Form XObject named `name` in `scope`.
///
/// Skips a non-form (e.g. image) XObject, a name absent from the scope, and a
/// re-entrant or too-deep reference (cycle/limit guard). A form's own
/// `/Resources` become the scope for its content; when it has none the parent
/// scope stands in, as the spec allows.
fn walk_xobject<'a>(
    doc: &'a Document,
    name: &[u8],
    scope: &Scope<'a>,
    depth: u8,
    inherited: Inherited<'a>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    if depth == 0 {
        return Ok(());
    }
    let Some(xobject_id) = scope_xobject_id(doc, scope, name) else {
        return Ok(());
    };
    // Cycle guard: an XObject already on the stack (or that we cannot read as a
    // stream) is skipped.
    if ctx.active.contains(&xobject_id) {
        return Ok(());
    }
    let Ok(Object::Stream(stream_obj)) = doc.get_object(xobject_id) else {
        return Ok(());
    };
    // Only Form XObjects carry content to walk; images are not text.
    if stream_obj
        .dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .ok()
        != Some(b"Form")
    {
        return Ok(());
    }
    let bytes = stream_obj
        .decompressed_content()
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("XObject content: {e}")))?;
    // Bound the decompressed XObject stream, as for page content: a Form XObject
    // is another place a decompression bomb could hide.
    if bytes.len() > ctx.max_page_bytes {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            format!(
                "XObject content is {} bytes, over the {}-byte limit",
                bytes.len(),
                ctx.max_page_bytes
            ),
        ));
    }
    let content = Content::decode(&bytes).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("decode XObject content: {e}"),
        )
    })?;

    // The form's own resources, or the parent scope when it declares none.
    let inner_resources = stream_obj
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|r| dict_or_ref(doc, r));
    let inner_scope = match inner_resources {
        Some(res) => Scope {
            inline: Some(res),
            referenced: &[],
        },
        None => *scope,
    };

    // Charge the total-walk budget for this form (only now that it is a real form
    // we will decode). Exhausting it means a page fans out to more walks than any
    // legitimate document, so fail closed rather than let the walk run away.
    let Some(remaining) = ctx.walks_remaining.checked_sub(1) else {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            format!(
                "page {} draws more than {MAX_XOBJECT_WALKS} Form XObjects; \
                 its text cannot be walked within the resource budget",
                ctx.page
            ),
        ));
    };
    ctx.walks_remaining = remaining;

    // A Form XObject may carry its own `/Matrix`, applied to its content on top
    // of the caller's CTM (ISO 32000-1 §8.10.1). Compose it so text the form
    // draws lands in the page's coordinate space; absent/malformed `/Matrix` is
    // the identity (no transform).
    let form_matrix = stream_obj
        .dict
        .get(b"Matrix")
        .ok()
        .and_then(|o| o.as_array().ok())
        .and_then(|a| read_matrix(a, 0))
        .unwrap_or(Matrix::IDENTITY);
    // Compose the form's `/Matrix` onto the inherited CTM, so text the form draws
    // lands in the page's coordinate space.
    let mut inherited = inherited;
    inherited.state.ctm = form_matrix.then(inherited.state.ctm);

    ctx.active.insert(xobject_id);
    let result = walk_stream(
        doc,
        &content,
        StreamTarget::XObject(xobject_id),
        &inner_scope,
        depth - 1,
        Some(inherited),
        ctx,
    );
    ctx.active.remove(&xobject_id);
    result
}

/// Collect the `/Font` entries of a resource scope (inline plus any referenced
/// resource dictionaries), name -> font dictionary.
fn scope_fonts<'a>(
    doc: &'a Document,
    scope: &Scope<'a>,
) -> BTreeMap<Vec<u8>, &'a lopdf::Dictionary> {
    let mut out = BTreeMap::new();
    let mut add = |resources: &'a lopdf::Dictionary| {
        if let Ok(font) = resources.get(b"Font")
            && let Some(font_dict) = dict_or_ref(doc, font)
        {
            for (name, value) in font_dict.iter() {
                let font = match value {
                    Object::Reference(id) => doc.get_dictionary(*id).ok(),
                    Object::Dictionary(dict) => Some(dict),
                    _ => None,
                };
                if let Some(font) = font {
                    out.entry(name.clone()).or_insert(font);
                }
            }
        }
    };
    if let Some(inline) = scope.inline {
        add(inline);
    }
    for &id in scope.referenced {
        if let Ok(resources) = doc.get_dictionary(id) {
            add(resources);
        }
    }
    out
}

/// Resolve the object id of the XObject named `name` in `scope`'s `/XObject`
/// dictionary, if it is an indirect reference.
fn scope_xobject_id(doc: &Document, scope: &Scope<'_>, name: &[u8]) -> Option<ObjectId> {
    let resource_dicts = scope.inline.into_iter().chain(
        scope
            .referenced
            .iter()
            .filter_map(|&id| doc.get_dictionary(id).ok()),
    );
    for resources in resource_dicts {
        if let Ok(xobjects) = resources.get(b"XObject")
            && let Some(xobjects) = dict_or_ref(doc, xobjects)
            && let Ok(entry) = xobjects.get(name)
            && let Ok(id) = entry.as_reference()
        {
            return Some(id);
        }
    }
    None
}

/// Resolve an object that is a dictionary, inline or by reference.
fn dict_or_ref<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

/// A resource dictionary scope a stream's `Tf`/`Do` names resolve in: an inline
/// dictionary and any referenced resource dictionaries (a page can inherit
/// several through its tree).
#[derive(Clone, Copy)]
struct Scope<'a> {
    inline: Option<&'a lopdf::Dictionary>,
    referenced: &'a [ObjectId],
}

/// The growing text, glyph map, and font table a walk appends to. Borrowed so a
/// single accumulation spans the page content and every Form XObject it draws.
struct TextAccum<'a> {
    text: &'a mut String,
    runs: &'a mut Vec<OffsetRun>,
    next_char: &'a mut usize,
    rects: &'a mut Vec<GlyphRect>,
    fonts: &'a mut Vec<FontEntry>,
}

/// The mutable state threaded through a page's content walk, unchanged as it
/// recurses into Form XObjects: the accumulating text/glyph/font output, the
/// XObjects currently on the recursion stack (cycle guard), the 1-based page
/// number for diagnostics, the per-stream decompression-bomb bound, and the
/// remaining total-walk budget.
struct Walk<'a> {
    accum: TextAccum<'a>,
    active: BTreeSet<ObjectId>,
    page: u32,
    /// Maximum decompressed size of any single content stream (page or XObject),
    /// refusing a decompression bomb.
    max_page_bytes: usize,
    /// Form XObject walks still permitted on this page. The stack-based `active`
    /// guard stops re-entry (cycles) but not re-walks: a form reached through
    /// several distinct paths is walked once per path, so a shallow tree that
    /// draws each child many times per level can fan out to a huge number of
    /// stream decodes without ever recursing cyclically. This bounds the total,
    /// independent of nesting depth.
    walks_remaining: u32,
}

#[cfg(test)]
mod tests {
    use lopdf::content::Operation;
    use lopdf::dictionary;

    use super::*;

    /// Build a one-page document whose single content stream is `ops`, drawing
    /// with a simple Helvetica font (no `/Widths`, so glyph advances are unknown —
    /// the gap-presence path), on a 100×200 point page. Returns the walked block.
    fn extract_block(ops: Vec<Operation>) -> super::TextBlock {
        extract_block_with_widths(ops, false)
    }

    /// As [`extract_block`], optionally giving the font a `/Widths` array (so glyph
    /// advances — and thus box widths — are known) and a descriptor with
    /// ascent/descent (so box heights are real rather than the default em box).
    fn extract_block_with_widths(ops: Vec<Operation>, widths: bool) -> super::TextBlock {
        let mut doc = Document::with_version("1.5");
        let mut font = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        };
        if widths {
            // Every byte 0..=255 is 500/1000 em wide — enough to give boxes a real
            // width without modelling real Helvetica metrics.
            let descriptor = doc.add_object(dictionary! {
                "Type" => "FontDescriptor",
                "FontName" => "Helvetica",
                "Ascent" => 718,
                "Descent" => -207,
            });
            font.set("FirstChar", 0);
            font.set("LastChar", 255);
            font.set("Widths", vec![Object::Integer(500); 256]);
            font.set("FontDescriptor", descriptor);
        }
        let font_id = doc.add_object(font);
        let resources = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content { operations: ops };
        let content_id = doc.add_object(lopdf::Stream::new(
            dictionary! {},
            content.encode().unwrap(),
        ));
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });

        text_block_for_page(&doc, 1, page_id, 10_000_000).unwrap()
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
        let geometry = super::super::PageGeometry {
            page: block.page,
            rects: block.rects.clone(),
            frame: block.frame,
        };
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
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    72.into(),
                    700.into(),
                ],
            ),
            show("Label:"),
            Operation::new("ET", vec![]),
            // Second field, a new text object positioned well to the right.
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    200.into(),
                    700.into(),
                ],
            ),
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
        let mut doc = Document::with_version("1.5");
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let font_res = dictionary! { "Font" => dictionary! { "F1" => font_id } };

        // The form's content: draw "Hi" at the form's own origin.
        let form_ops = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 10.into()]),
                Operation::new("Td", vec![0.into(), 0.into()]),
                Operation::new("Tj", vec![Object::string_literal("Hi")]),
                Operation::new("ET", vec![]),
            ],
        };
        // The form carries a /Matrix translating it to (20, 180) on the page.
        let form_id = doc.add_object(lopdf::Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 20.into(), 180.into()],
                "Resources" => font_res.clone(),
            },
            form_ops.encode().unwrap(),
        ));

        // The page just invokes the form.
        let page_content = Content {
            operations: vec![Operation::new("Do", vec!["Fm".into()])],
        };
        let content_id = doc.add_object(lopdf::Stream::new(
            dictionary! {},
            page_content.encode().unwrap(),
        ));
        let resources = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Fm" => form_id },
        });
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });

        let block = text_block_for_page(&doc, 1, page_id, 10_000_000).unwrap();
        // The form's text is extracted (inheritance did not drop it).
        assert_eq!(block.text, "Hi");
        // And it is positioned by the inherited transform: the glyph box sits
        // near the form's translated origin x = 20/100 = 0.2, not the content's
        // local x = 0.
        let geometry = super::super::PageGeometry {
            page: block.page,
            rects: block.rects.clone(),
            frame: block.frame,
        };
        let rects = geometry.rects_for(0..block.text.chars().count());
        assert_eq!(rects.len(), 1, "one line: {rects:?}");
        assert!(
            (rects[0].x - 0.2).abs() < 1e-6,
            "inherited translate: {:?}",
            rects[0]
        );
    }
}
