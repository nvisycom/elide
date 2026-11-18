//! One content stream walked in operator order: the graphics-state stack,
//! text positioning, and the operators that show text or draw a form or
//! pattern.

use std::collections::BTreeMap;
use std::slice;

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object};

use super::geometry::{Cursor, TextState};
use super::nested::{select_pattern, walk_xobject};
use super::{TextRun, Walk};
use crate::content::text::{FontEntry, StreamTarget};
use crate::pdf::reader::font::{FontText, GlyphMetrics, WidthSource};
use crate::pdf::reader::{DocumentExt, Matrix, ObjectExt, Scope};

/// The numeric operand at `index` as `f64`, or `0.0` when absent or non-numeric
/// (a malformed operator reads as its zero, never a hard error — positioning is
/// best-effort for spacing).
pub(super) fn operand_f64(operands: &[Object], index: usize) -> f64 {
    operands
        .get(index)
        .map(|o| o.as_number().unwrap_or(0.0))
        .unwrap_or(0.0)
}

/// Read six numeric operands from `base` as a [`Matrix`] `[a b c d e f]`, or
/// `None` when fewer than six are present.
pub(super) fn read_matrix(operands: &[Object], base: usize) -> Option<Matrix> {
    (operands.len() >= base + 6)
        .then(|| Matrix::from(std::array::from_fn(|i| operand_f64(operands, base + i))))
}

/// The caller's graphics state passed into a `Do`-invoked Form XObject: the
/// font it had selected, its parametric text state (with the form's `/Matrix`
/// already composed onto the CTM), and the pen position, so the form's text is
/// positioned in the page's coordinate space and a word gap across the boundary
/// is still judged.
pub(super) struct Inherited<'a> {
    /// The caller's selected font, or `None` when it had selected none. Carries
    /// everything a form needs to draw text before its own `Tf`: the table slot,
    /// the resolved encoding, and the advance/metric tables that position glyphs.
    pub(super) font: Option<InheritedFont<'a>>,
    /// The caller's text state, CTM composed with the form's `/Matrix`.
    pub(super) state: TextState,
}

/// A graphics-state snapshot pushed by `q` and restored by `Q`: the parametric
/// text state plus the selected font (encoding, table slot, and advance/metric
/// tables), since the font is part of the graphics state.
pub(super) struct SavedState<'a> {
    pub(super) state: TextState,
    pub(super) current: Option<&'a Option<FontText<'a>>>,
    pub(super) font_slot: u16,
    pub(super) widths: &'a WidthSource,
    pub(super) metrics: GlyphMetrics,
}

/// The caller's selected font, inherited by a `Do`-invoked Form XObject so text
/// it draws before its own `Tf` is decoded *and positioned* with that font.
pub(super) struct InheritedFont<'a> {
    /// The font-table slot (routes a deleted glyph to the right `/ToUnicode`).
    pub(super) slot: u16,
    /// The resolved encoding (`&Some` decodable, `&Some(None)` undecodable).
    pub(super) encoding: &'a Option<FontText<'a>>,
    /// The font's advance widths, so glyph advances (and spacing) are right.
    pub(super) widths: &'a WidthSource,
    /// The font's vertical metrics, so glyph box heights are right.
    pub(super) metrics: GlyphMetrics,
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
pub(super) fn walk_stream<'a>(
    doc: &'a Document,
    content: &Content,
    stream: StreamTarget,
    scope: &Scope<'a>,
    depth: u8,
    inherited: Option<Inherited<'_>>,
    ctx: &mut Walk<'_, 'a>,
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
            // text included, wherever it paints. Selecting it paints nothing,
            // so its text is read once the page's is, never inside it.
            "scn" | "SCN" => {
                if let Some(Object::Name(name)) = op.operands.last() {
                    select_pattern(doc, name, scope, depth, ctx);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
