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
use std::slice;

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use super::geometry::{
    Cursor, DeviceRect, GlyphRect, Matrix, PageFrame, SpaceRule, TextState, page_frame,
};
use super::glyphs::{FontText, decode_glyphs};
use super::widths::{GlyphMetrics, WidthSource};
use super::{Address, FontEntry, GlyphBytes, OffsetMap, OffsetRun, StreamTarget, TextBlock};
use crate::document::Store;
use crate::document::object::{DictionaryExt, DocumentExt, ObjectExt, StreamExt};
use crate::document::scope::Scope;

/// The character code of the single-byte space, the only code `Tw` word spacing
/// applies to (ISO 32000-1 §9.3.3).
const SPACE_CODE: u32 = 32;

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
        enc: &FontText<'_>,
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
                                let kern = item.as_number().unwrap_or(0.0);
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
    fn show_string(&mut self, enc: &FontText<'_>, bytes: &[u8], address: Address) -> Result<()> {
        for glyph in decode_glyphs(enc, bytes)? {
            let origin = self.cursor.device_origin(self.state.ctm);
            // The gap threshold is a fraction of the font size *in device units*,
            // measured along the glyph-advance direction — so it holds under
            // rotation / shear, not just upright text. The axis scale carries one
            // text unit to the page; its sign follows `font_size · h_scale`, which
            // is negative for mirrored text (negative `Tz`), so the advance runs
            // *backwards* along the matrix's x-basis: flip the axis then, and take
            // the magnitude for `em`. Guard a degenerate transform so `em` stays
            // positive.
            let (mut axis, scale) = self.cursor.writing_axis(self.state.ctm);
            let signed = self.state.font_size * self.state.h_scale;
            if signed < 0.0 {
                axis = (-axis.0, -axis.1);
            }
            let em = (signed.abs() * scale).max(f64::MIN_POSITIVE);
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
    operands
        .get(index)
        .map(|o| o.as_number().unwrap_or(0.0))
        .unwrap_or(0.0)
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
pub(crate) fn text_blocks(store: &Store) -> Result<Vec<TextBlock>> {
    let doc = store.doc();
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
/// The per-page primitive [`text_blocks`] loops over.
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
    let mut fonts: Vec<FontEntry> = Vec::new();
    let mut ctx = Walk {
        accum: TextAccum {
            text: &mut text,
            runs: &mut runs,
            next_char: &mut next_char,
            rects: &mut rects,
            fonts: &mut fonts,
        },
        pen: None,
        active: BTreeSet::new(),
        patterns: BTreeSet::new(),
        page,
        walks_remaining: MAX_XOBJECT_WALKS,
    };

    walk_stream(
        doc,
        &content,
        StreamTarget::PageContent,
        &Scope::page(doc, page_id),
        MAX_XOBJECT_DEPTH,
        None,
        &mut ctx,
    )?;

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
        frame,
        fonts,
    })
}

/// The caller's graphics state passed into a `Do`-invoked Form XObject: the
/// font it had selected, its parametric text state (with the form's `/Matrix`
/// already composed onto the CTM), and the pen position, so the form's text is
/// positioned in the page's coordinate space and a word gap across the boundary
/// is still judged.
struct Inherited<'a> {
    /// The caller's selected font, or `None` when it had selected none. Carries
    /// everything a form needs to draw text before its own `Tf`: the table slot,
    /// the resolved encoding, and the advance/metric tables that position glyphs.
    font: Option<InheritedFont<'a>>,
    /// The caller's text state, CTM composed with the form's `/Matrix`.
    state: TextState,
}

/// A graphics-state snapshot pushed by `q` and restored by `Q`: the parametric
/// text state plus the selected font (encoding, table slot, and advance/metric
/// tables), since the font is part of the graphics state.
struct SavedState<'a> {
    state: TextState,
    current: Option<&'a Option<FontText<'a>>>,
    font_slot: u16,
    widths: &'a WidthSource,
    metrics: GlyphMetrics,
}

/// The caller's selected font, inherited by a `Do`-invoked Form XObject so text
/// it draws before its own `Tf` is decoded *and positioned* with that font.
struct InheritedFont<'a> {
    /// The font-table slot (routes a deleted glyph to the right `/ToUnicode`).
    slot: u16,
    /// The resolved encoding (`&Some` decodable, `&Some(None)` undecodable).
    encoding: &'a Option<FontText<'a>>,
    /// The font's advance widths, so glyph advances (and spacing) are right.
    widths: &'a WidthSource,
    /// The font's vertical metrics, so glyph box heights are right.
    metrics: GlyphMetrics,
}

/// Walk one content stream in operator order, appending its text and glyph map
/// to `accum`, and recursing into each Form XObject drawn with `Do`.
///
/// `scope` is the resource dictionary a `Tf`/`Do` resolves names in. `stream` is
/// stamped onto every glyph so a deletion edits the correct stream. `active`
/// holds the XObjects currently on the recursion stack (cycle guard); `depth`
/// bounds nesting. Text under an undecodable font fails closed, as at page level.
///
/// `inherited` is the caller's graphics state at the `Do` that invoked this
/// stream: a Form XObject inherits the graphics state, so text it draws before
/// its own operators uses the caller's font *and* transform. It carries the
/// font-table slot + resolved encoding and the caller's [`TextState`] (whose CTM
/// already has the form's `/Matrix` composed in). `None` at page level, where
/// the state starts fresh.
fn walk_stream<'a>(
    doc: &'a Document,
    content: &Content,
    stream: StreamTarget,
    scope: &Scope<'a>,
    depth: u8,
    inherited: Option<Inherited<'_>>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    // Resolve this scope's fonts to encodings, width tables, and a `FontEntry`
    // per font in the shared table. A local name->slot map points the run at the
    // right entry on each `Tf`, and name->encoding / name->widths carry what a
    // `Tf` then needs to decode and position that font's glyphs.
    let font_dicts: Vec<(&[u8], &Dictionary)> = scope
        .entries(doc, b"Font")
        .into_iter()
        .filter_map(|(name, font)| Some((name, doc.resolve_dict(font)?)))
        .collect();
    let mut font_slot: BTreeMap<Vec<u8>, u16> = BTreeMap::new();
    let mut encodings: BTreeMap<Vec<u8>, Option<FontText>> = BTreeMap::new();
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
            return Err(Error::redaction(format!(
                "page {} references more than {} fonts across its content \
                     streams; its text cannot be safely redacted",
                ctx.page,
                u16::MAX,
            )));
        };
        ctx.accum.fonts.push(FontEntry { to_unicode });
        font_slot.insert(name.to_vec(), slot);
        encodings.insert(name.to_vec(), FontText::of(doc, font)?);
        widths.insert(name.to_vec(), WidthSource::from_font(doc, font));
        metrics.insert(name.to_vec(), GlyphMetrics::from_font(doc, font));
    }

    // The current font's resolved encoding: `None` before any `Tf` (unless a
    // font is inherited from the caller's graphics state, below), or `Some(None)`
    // when the selected font could not be decoded or names a font absent from
    // this scope's resources.
    const UNRESOLVED: &Option<FontText> = &None;
    const NO_WIDTHS: &WidthSource = &WidthSource::Unknown;
    // A Form XObject inherits the caller's selected font — its encoding, advance
    // widths, and metrics — so text it draws before its own `Tf` is decoded and
    // positioned with that font. At page level there is nothing to inherit. A
    // local `Tf` later replaces all of these.
    let inherited_font = inherited.as_ref().and_then(|inh| inh.font.as_ref());
    let (mut current, mut font_slot_current): (Option<&Option<FontText>>, u16) =
        match inherited_font {
            Some(f) => (Some(f.encoding), f.slot),
            None => (None, 0),
        };
    let mut width_current: &WidthSource = inherited_font.map_or(NO_WIDTHS, |f| f.widths);
    let mut metrics_current = inherited_font.map_or_else(GlyphMetrics::default, |f| f.metrics);

    // The text geometry: the matrix cursor (reset at `BT`) and the parametric
    // text state with its `q`/`Q` save stack. The pen (where the last glyph left
    // off) is shared across this stream and every Form XObject it draws — one
    // cell threaded through the recursion — so a word gap is judged consistently
    // across a `Do` boundary in either direction. A form inherits the caller's
    // text state (its CTM already carries the form's `/Matrix`).
    let mut cursor = Cursor::new();
    let mut state = inherited
        .as_ref()
        .map_or_else(TextState::default, |inh| inh.state);
    // The `q`/`Q` graphics-state stack. The selected font is part of the
    // graphics state (ISO 32000-1 §8.4), so a save/restore carries it — the
    // encoding, font slot, and the advance/metric tables — alongside the
    // parametric `TextState`. The text matrix is *not* graphics state (it resets
    // at `BT`), so it is not saved.
    let mut gs_stack: Vec<SavedState<'_>> = Vec::new();

    for (op_idx, op) in content.operations.iter().enumerate() {
        match op.operator.as_str() {
            // Graphics-state save/restore. A `Q` with an empty stack is a
            // malformed balance we ignore.
            "q" => gs_stack.push(SavedState {
                state,
                current,
                font_slot: font_slot_current,
                widths: width_current,
                metrics: metrics_current,
            }),
            "Q" => {
                if let Some(saved) = gs_stack.pop() {
                    state = saved.state;
                    current = saved.current;
                    font_slot_current = saved.font_slot;
                    width_current = saved.widths;
                    metrics_current = saved.metrics;
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
                                Some(s) => (slice::from_ref(s), 2),
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
                            pen: &mut ctx.pen,
                            state,
                            widths: width_current,
                            metrics: metrics_current,
                            stream,
                            font: font_slot_current,
                        };
                        run.show(enc, operands, op_idx, base)?;
                    }
                    Some(None) => {
                        return Err(Error::redaction(format!(
                            "page {} draws text with a font whose encoding \
                                 could not be decoded or was not found in the page \
                                 resources; its text cannot be redacted",
                            ctx.page
                        )));
                    }
                    None => {
                        return Err(Error::redaction(format!(
                            "page {} draws text before selecting a font; \
                             its text cannot be decoded for redaction",
                            ctx.page
                        )));
                    }
                }
            }

            // A `Do` may draw a Form XObject, whose content draws text in its own
            // stream and resources. Recurse so that text is located and
            // redactable too.
            "Do" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    // Pass the current graphics state down: the form inherits the
                    // selected font and the text state (its CTM, onto which
                    // `walk_xobject` composes the form's `/Matrix`). The pen cell
                    // is shared, so the form's text updates it and a gap right
                    // after the `Do` is judged against where the form left off.
                    let inherited = Inherited {
                        font: current.map(|encoding| InheritedFont {
                            slot: font_slot_current,
                            encoding,
                            widths: width_current,
                            metrics: metrics_current,
                        }),
                        state,
                    };
                    walk_xobject(doc, name, scope, depth, inherited, ctx)?;
                }
            }

            // A tiling pattern selected to paint with draws its cell's content,
            // text included.
            "scn" | "SCN" => {
                if let Some(Object::Name(name)) = op.operands.last() {
                    walk_pattern(doc, name, scope, depth, ctx)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve and recurse into the Form XObject named `name` in `scope`.
///
/// Skips a non-form (e.g. image) XObject and a name absent from the scope,
/// neither of which draws text. A form's own `/Resources` become the scope for
/// its content; when it has none the parent scope stands in, as the spec
/// allows.
fn walk_xobject<'a>(
    doc: &'a Document,
    name: &[u8],
    scope: &Scope<'a>,
    depth: u8,
    inherited: Inherited<'a>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    let Some(id) = scope
        .lookup(doc, b"XObject", name)
        .and_then(|xobject| xobject.as_reference().ok())
    else {
        return Ok(());
    };
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return Ok(());
    };
    // Only Form XObjects carry content to walk; images are not text.
    if !stream.dict.is_subtype(doc, b"Form") {
        return Ok(());
    }
    walk_nested(doc, id, stream, scope, depth, inherited, ctx)
}

/// Resolve and recurse into the tiling pattern named `name` in `scope`, the
/// pattern a `scn`/`SCN` selects to paint with.
///
/// A pattern's cell is drawn in its own graphics state, in the page's default
/// space transformed by the pattern's `/Matrix` (8.7.3.1), so it inherits no
/// font or transform. Its text is located once, however many times the cell is
/// tiled or the pattern selected, and deleting it from the one stream removes
/// it from every tile. A shading pattern draws no text.
fn walk_pattern<'a>(
    doc: &'a Document,
    name: &[u8],
    scope: &Scope<'a>,
    depth: u8,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    let Some(id) = scope
        .lookup(doc, b"Pattern", name)
        .and_then(|pattern| pattern.as_reference().ok())
    else {
        return Ok(());
    };
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return Ok(());
    };
    if stream
        .dict
        .get(b"PatternType")
        .ok()
        .and_then(ObjectExt::as_number)
        != Some(1.0)
        || !ctx.patterns.insert(id)
    {
        return Ok(());
    }
    let inherited = Inherited {
        font: None,
        state: TextState::default(),
    };
    walk_nested(doc, id, stream, scope, depth, inherited, ctx)
}

/// Walk the content of the form or tiling pattern `stream` (object `id`),
/// drawn from `scope` under the `inherited` state, its `/Matrix` composed onto
/// that state's transform.
///
/// Skips a re-entrant reference (cycle guard); content nested past
/// [`MAX_XOBJECT_DEPTH`], or more walks than [`MAX_XOBJECT_WALKS`], is an
/// error.
fn walk_nested<'a>(
    doc: &'a Document,
    id: ObjectId,
    stream: &'a Stream,
    scope: &Scope<'a>,
    depth: u8,
    mut inherited: Inherited<'a>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    if ctx.active.contains(&id) {
        return Ok(());
    }
    if depth == 0 {
        return Err(Error::resource_limit(format!(
            "page {} nests forms and patterns deeper than {MAX_XOBJECT_DEPTH} levels; \
             its text cannot be walked",
            ctx.page
        )));
    }
    let content = Content::decode(&stream.decoded()?)
        .map_err(|e| Error::malformed(format!("decode XObject content: {e}")))?;

    // Its own resources, or the parent scope when it declares none.
    let inner_scope = scope.inner(doc, &stream.dict);

    // Charge the total-walk budget. Exhausting it means a page fans out to
    // more walks than any legitimate document, so fail closed rather than let
    // the walk run away.
    let Some(remaining) = ctx.walks_remaining.checked_sub(1) else {
        return Err(Error::resource_limit(format!(
            "page {} draws more than {MAX_XOBJECT_WALKS} forms and patterns; \
             its text cannot be walked within the resource budget",
            ctx.page
        )));
    };
    ctx.walks_remaining = remaining;

    // The `/Matrix` maps its content into the space it is drawn in (8.10.1,
    // 8.7.3.1); absent or malformed, it is the identity.
    let matrix = stream
        .dict
        .get(b"Matrix")
        .ok()
        .and_then(|o| o.as_array().ok())
        .and_then(|a| read_matrix(a, 0))
        .unwrap_or(Matrix::IDENTITY);
    inherited.state.ctm = matrix.then(inherited.state.ctm);

    ctx.active.insert(id);
    let result = walk_stream(
        doc,
        &content,
        StreamTarget::XObject(id),
        &inner_scope,
        depth - 1,
        Some(inherited),
        ctx,
    );
    ctx.active.remove(&id);
    result
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
/// recurses into Form XObjects: the accumulating text/glyph/font output, the pen
/// position shared across the page and every form it draws (so a word gap is
/// judged consistently across a `Do` boundary), the XObjects currently on the
/// recursion stack (cycle guard), the 1-based page number for diagnostics, and the
/// remaining total-walk budget.
struct Walk<'a> {
    accum: TextAccum<'a>,
    /// Where the last shown glyph left the pen, in device space, shared across
    /// every stream of the page (`None` before the first glyph).
    pen: Option<(f64, f64)>,
    active: BTreeSet<ObjectId>,
    /// The tiling patterns already walked on this page: a pattern's walk does
    /// not depend on where it is selected, so each is walked once.
    patterns: BTreeSet<ObjectId>,
    page: u32,
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
    use elide_core::ErrorKind;
    use lopdf::content::Operation;
    use lopdf::{Dictionary, dictionary};

    use super::*;
    use crate::fixtures::with_pages;

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
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content_id,
                "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
            }],
        )[0];

        text_block_for_page(&doc, 1, page_id).unwrap()
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
        let mut chars: Vec<usize> = geometry
            .glyphs_in(&regions)
            .flat_map(|range| range.collect::<Vec<_>>())
            .collect();
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
        let form_id = doc.add_object(Stream::new(
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
        let content_id =
            doc.add_object(Stream::new(dictionary! {}, page_content.encode().unwrap()));
        let resources = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Fm" => form_id },
        });
        let page_id = with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content_id,
                "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
            }],
        )[0];

        let block = text_block_for_page(&doc, 1, page_id).unwrap();
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
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    90.into(),
                    100.into(),
                ],
            ),
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

    /// A Form XObject that draws text *before its own `Tf`* uses the inherited
    /// font's advance widths, so the glyph box has a real width (not the
    /// zero-advance fallback of an uninherited font).
    #[test]
    fn form_xobject_inherits_font_widths() {
        let mut doc = Document::with_version("1.5");
        // A font with explicit widths (500/1000 for every byte) and a descriptor.
        let descriptor = doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => "Helvetica",
            "Ascent" => 718, "Descent" => -207,
        });
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "FirstChar" => 0, "LastChar" => 255,
            "Widths" => vec![Object::Integer(500); 256],
            "FontDescriptor" => descriptor,
        });
        let font_res = dictionary! { "Font" => dictionary! { "F1" => font_id } };

        // The form draws text with no `Tf` of its own — it relies on the font the
        // caller selected before the `Do`.
        let form_ops = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Td", vec![10.into(), 100.into()]),
                Operation::new("Tj", vec![Object::string_literal("Hi")]),
                Operation::new("ET", vec![]),
            ],
        };
        let form_id = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "Resources" => font_res.clone() },
            form_ops.encode().unwrap(),
        ));

        // The page selects F1 at size 20, then invokes the form.
        let page_content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 20.into()]),
                Operation::new("ET", vec![]),
                Operation::new("Do", vec!["Fm".into()]),
            ],
        };
        let content_id =
            doc.add_object(Stream::new(dictionary! {}, page_content.encode().unwrap()));
        let resources = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
            "XObject" => dictionary! { "Fm" => form_id },
        });
        let page_id = with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content_id,
                "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
            }],
        )[0];

        let block = text_block_for_page(&doc, 1, page_id).unwrap();
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
        let mut doc = Document::with_version("1.5");
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let font_res = dictionary! { "Font" => dictionary! { "F1" => font_id } };

        // The form draws "Beta" at x=140 (a wide gap after the page's "Alpha").
        let form_ops = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new(
                    "Tm",
                    vec![
                        1.into(),
                        0.into(),
                        0.into(),
                        1.into(),
                        140.into(),
                        150.into(),
                    ],
                ),
                Operation::new("Tj", vec![Object::string_literal("Beta")]),
                Operation::new("ET", vec![]),
            ],
        };
        let form_id = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "Resources" => font_res.clone() },
            form_ops.encode().unwrap(),
        ));

        // The page: "Alpha" at x=10, Do the form, then "Gamma" at x=300 (a wide
        // gap after the form's "Beta" ended around x≈160).
        let page_content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new(
                    "Tm",
                    vec![
                        1.into(),
                        0.into(),
                        0.into(),
                        1.into(),
                        10.into(),
                        150.into(),
                    ],
                ),
                Operation::new("Tj", vec![Object::string_literal("Alpha")]),
                Operation::new("ET", vec![]),
                Operation::new("Do", vec!["Fm".into()]),
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new(
                    "Tm",
                    vec![
                        1.into(),
                        0.into(),
                        0.into(),
                        1.into(),
                        300.into(),
                        150.into(),
                    ],
                ),
                Operation::new("Tj", vec![Object::string_literal("Gamma")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id =
            doc.add_object(Stream::new(dictionary! {}, page_content.encode().unwrap()));
        let resources = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
            "XObject" => dictionary! { "Fm" => form_id },
        });
        let page_id = with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content_id,
                "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 400.into(), 200.into()],
            }],
        )[0];

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
        let mut doc = Document::with_version("1.5");
        let descriptor = doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => "F",
            "Ascent" => 700, "Descent" => -200,
        });
        // F1 is wide (900/1000 per glyph), F2 is narrow (100/1000).
        let wide = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Wide",
            "FirstChar" => 0, "LastChar" => 255,
            "Widths" => vec![Object::Integer(900); 256],
            "FontDescriptor" => descriptor,
        });
        let narrow = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Narrow",
            "FirstChar" => 0, "LastChar" => 255,
            "Widths" => vec![Object::Integer(100); 256],
            "FontDescriptor" => descriptor,
        });

        // Select F1 (wide), q, select F2 (narrow), Q, then draw "AB".
        let page_content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new(
                    "Tm",
                    vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 150.into()],
                ),
                Operation::new("Tf", vec!["F1".into(), 10.into()]),
                Operation::new("q", vec![]),
                Operation::new("Tf", vec!["F2".into(), 10.into()]),
                Operation::new("Q", vec![]),
                Operation::new("Tj", vec![Object::string_literal("AB")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id =
            doc.add_object(Stream::new(dictionary! {}, page_content.encode().unwrap()));
        let resources = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => wide, "F2" => narrow },
        });
        let page_id = with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content_id,
                "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
            }],
        )[0];

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
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let mut entries = Dictionary::new();
        for (i, id) in xobjects(&mut doc).into_iter().enumerate() {
            entries.set(format!("X{i}"), id);
        }
        let resources = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font },
            "XObject" => entries,
        });
        let content = doc.add_object(Stream::new(
            dictionary! {},
            Content { operations: ops }.encode().unwrap(),
        ));
        let page_id = with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
            }],
        )[0];
        (doc, page_id)
    }

    /// Text shown with `/F1`, the font the page resources name.
    fn hello() -> Vec<Operation> {
        vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Object::string_literal("Hello")]),
            Operation::new("ET", vec![]),
        ]
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
                hello(),
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
        let (mut doc, page_id) = page_drawing(hello(), |_| Vec::new());
        let content = doc.get_page_contents(page_id)[0];
        doc.objects.remove(&content);
        assert!(text_block_for_page(&doc, 1, page_id).is_err());
    }
}
