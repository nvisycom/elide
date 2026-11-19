//! One content stream walked in operator order: the graphics-state stack,
//! text positioning, and the operators that show text or draw a form or
//! pattern.

use std::collections::BTreeMap;
use std::slice;

use elide_core::{Error, Result};

use super::geometry::{Cursor, TextState};
use super::paint::{Paint, Painting};
use super::{TextRun, Walk};
use crate::content::text::{FontEntry, StreamTarget};
use crate::pdf::content::Operation;
use crate::pdf::object::Value;
use crate::pdf::reader::font::{FontText, GlyphMetrics, WidthSource};
use crate::pdf::reader::{DeviceRect, DictExt, GlyphRect, Matrix, Scope};

/// The numeric operand at `index` as `f64`, or `0.0` when absent or non-numeric
/// (a malformed operator reads as its zero, never a hard error — positioning is
/// best-effort for spacing).
pub(super) fn operand_f64(operands: &[Value], index: usize) -> f64 {
    operands
        .get(index)
        .and_then(Value::as_number)
        .unwrap_or(0.0)
}

/// Read six numeric operands from `base` as a [`Matrix`] `[a b c d e f]`, or
/// `None` when fewer than six are present.
pub(super) fn read_matrix(operands: &[Value], base: usize) -> Option<Matrix> {
    (operands.len() >= base + 6)
        .then(|| Matrix::from(std::array::from_fn(|i| operand_f64(operands, base + i))))
}

/// A font selected to show text with, part of the graphics state: its slot in
/// the page's font table (which routes a deleted glyph's code to the right
/// `/ToUnicode`), how its codes read (`None` when they can't be, or the name
/// selected names no font in scope), and its advance widths and vertical
/// metrics, which position and size its glyphs.
#[derive(Clone, Copy)]
pub(super) struct Font<'f> {
    pub(super) slot: u16,
    pub(super) text: Option<&'f FontText>,
    pub(super) widths: &'f WidthSource,
    pub(super) metrics: GlyphMetrics,
    /// Whether its glyphs are outlines a clipping rendering mode clips to;
    /// a Type3 font's are procedures, which no such mode affects (9.3.6).
    pub(super) outlines: bool,
}

/// The fonts a stream's scope names, each with its slot in the page's font
/// table.
struct Fonts {
    by_name: BTreeMap<Vec<u8>, ScopeFont>,
}

/// One font a scope names, read.
struct ScopeFont {
    slot: u16,
    text: Option<FontText>,
    widths: WidthSource,
    metrics: GlyphMetrics,
    outlines: bool,
}

impl Fonts {
    /// The font `name` selects, or, when the scope names no such font, one
    /// whose codes can't be read, keeping the slot of `previous`.
    fn select(&self, name: &[u8], previous: Option<Font<'_>>) -> Font<'_> {
        const NO_WIDTHS: &WidthSource = &WidthSource::Unknown;
        match self.by_name.get(name) {
            Some(font) => Font {
                slot: font.slot,
                text: font.text.as_ref(),
                widths: &font.widths,
                metrics: font.metrics,
                outlines: font.outlines,
            },
            None => Font {
                slot: previous.map_or(0, |font| font.slot),
                text: None,
                widths: NO_WIDTHS,
                metrics: GlyphMetrics::default(),
                outlines: true,
            },
        }
    }
}

/// The caller's graphics state passed into a `Do`-invoked Form XObject: the
/// font it had selected, its parametric text state (with the form's
/// `/Matrix` already composed onto the CTM), and what it paints with, so the
/// form's text is decoded with that font before its own `Tf`, positioned in
/// the page's space, and a fill it makes before its own colour paints the
/// caller's pattern.
pub(super) struct Inherited<'a> {
    /// The caller's selected font, or `None` when it had selected none.
    pub(super) font: Option<Font<'a>>,
    /// The caller's text state, CTM composed with the form's `/Matrix`.
    pub(super) state: TextState,
    /// The caller's patterns, clip and line style.
    pub(super) painting: Painting,
}

/// A graphics-state snapshot pushed by `q` and restored by `Q`: the parametric
/// text state, the patterns painted with, and the selected font, all graphics
/// state (ISO 32000-1 §8.4). The text matrix is *not* graphics state (it
/// resets at `BT`), so it is not saved.
struct SavedState<'a> {
    state: TextState,
    painting: Painting,
    font: Option<Font<'a>>,
}

/// What a content stream is walked with: the stream its glyphs are stamped
/// with, so a deletion edits the right one; the scope its names resolve in;
/// the nesting depth left; and the caller's graphics state at the `Do` that
/// drew it (`None` at page level, where the state starts fresh).
pub(super) struct Frame<'s, 'i, 'd> {
    pub(super) stream: StreamTarget,
    pub(super) scope: &'s Scope<'d>,
    pub(super) depth: u8,
    pub(super) inherited: Option<Inherited<'i>>,
}

impl<'d> Walk<'_, 'd> {
    /// The fonts `scope` names, each read and given a slot in the page's font
    /// table.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if the table
    /// would outgrow its `u16` slots, so a deleted glyph's code could be
    /// routed to the wrong `/ToUnicode`; or as [`FontText::of`].
    fn fonts(&mut self, scope: &Scope<'d>) -> Result<Fonts> {
        let mut by_name = BTreeMap::new();
        for (name, entry) in scope.entries(b"Font") {
            let Some(font) = entry.value.into_dict() else {
                continue;
            };
            let to_unicode = font.reference(b"ToUnicode");
            // Every stream walk appends its scope's fonts, so the table can
            // grow past the `u16` a glyph records its font by; fail closed
            // rather than let a slot truncate and misroute the scrub.
            let Ok(slot) = u16::try_from(self.accum.fonts.len()) else {
                return Err(Error::redaction(format!(
                    "page {} references more than {} fonts across its content \
                     streams; its text cannot be safely redacted",
                    self.page,
                    u16::MAX,
                )));
            };
            self.accum.fonts.push(FontEntry { to_unicode });
            by_name.insert(
                name.to_vec(),
                ScopeFont {
                    slot,
                    text: FontText::of(&font)?,
                    widths: WidthSource::from_font(&font),
                    metrics: GlyphMetrics::from_font(&font),
                    outlines: !font.is_subtype(b"Type3"),
                },
            );
        }
        Ok(Fonts { by_name })
    }

    /// Walk the content stream `content`, as `frame` gives it, in operator
    /// order, appending its text and glyph map, recursing into each Form
    /// XObject it draws with `Do`, and selecting the tiling patterns it paints
    /// with. A form inherits the caller's graphics state, so text it draws
    /// before its own operators uses the caller's font and transform. Text
    /// under a font whose codes can't be read fails closed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if text is
    /// drawn before a font is selected or in a font whose codes can't be
    /// read; as [`fonts`](Self::fonts), [`TextRun::show`] and
    /// [`walk_xobject`](Self::walk_xobject).
    pub(super) fn walk_stream(
        &mut self,
        content: &[Operation],
        frame: Frame<'_, '_, 'd>,
    ) -> Result<()> {
        let Frame {
            stream,
            scope,
            depth,
            inherited,
        } = frame;
        let fonts = self.fonts(scope)?;
        // The selected font: the caller's, if any, until a `Tf` here selects
        // one of this scope's.
        let mut font: Option<Font<'_>> = inherited.as_ref().and_then(|inh| inh.font);
        // The text geometry: the matrix cursor (reset at `BT`) and the
        // parametric text state, the caller's at a `Do` (its CTM already
        // carries the form's `/Matrix`). The pen, where the last glyph left
        // off, is shared across this stream and every form it draws, so a word
        // gap is judged consistently across a `Do` boundary.
        let mut cursor = Cursor::new();
        let mut state = inherited
            .as_ref()
            .map_or_else(TextState::default, |inh| inh.state);
        let mut gs_stack: Vec<SavedState<'_>> = Vec::new();
        // The patterns fills and strokes paint with, the clip, and the path
        // being built, so the region each pattern paints is known; and the
        // space a pattern this stream selects is laid out in (8.7.3.1).
        let mut painting = inherited
            .as_ref()
            .map_or_else(Painting::default, |inh| inh.painting.inherited());
        let pattern_space = state.ctm;

        for (op_idx, op) in content.iter().enumerate() {
            if let Some(painted) = painting.path(op, state.ctm) {
                self.paint(painted);
                continue;
            }
            match op.operator.as_str() {
                // Graphics-state save/restore. A `Q` with an empty stack is a
                // malformed balance we ignore.
                "q" => gs_stack.push(SavedState {
                    state,
                    painting,
                    font,
                }),
                "Q" => {
                    if let Some(saved) = gs_stack.pop() {
                        state = saved.state;
                        painting.restore(saved.painting);
                        font = saved.font;
                    }
                }
                "cm" => {
                    if let Some(m) = read_matrix(&op.operands, 0) {
                        state.ctm = m.then(state.ctm);
                    }
                }

                // Text object: the text and line matrices reset to the
                // identity (ISO 32000-1 §9.4.1). The pen is kept across the
                // boundary: it is a device-space position, unaffected by the
                // matrix reset, so a gap between one text object's last glyph
                // and the next object's first is judged by the same rule. Many
                // producers emit one `BT`/`ET` per positioned field, so nulling
                // the pen here would run every field into the next.
                "BT" => {
                    cursor = Cursor::new();
                    painting.begin_text();
                }
                "ET" => painting.end_text(),

                // Text state parameters.
                "Tc" => state.char_spacing = operand_f64(&op.operands, 0),
                "Tw" => state.word_spacing = operand_f64(&op.operands, 0),
                "Tz" => state.h_scale = operand_f64(&op.operands, 0) / 100.0,
                "TL" => state.leading = operand_f64(&op.operands, 0),
                "Ts" => state.rise = operand_f64(&op.operands, 0),
                "Tf" => {
                    if let Some(Value::Name(name)) = op.operands.first() {
                        font = Some(fonts.select(name, font));
                    }
                    state.font_size = operand_f64(&op.operands, 1);
                }

                // Text positioning: move the line/text matrix.
                "Td" => cursor
                    .next_line_offset(operand_f64(&op.operands, 0), operand_f64(&op.operands, 1)),
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

                // Text showing. `'` is an implicit `T*` then show; `"`
                // additionally sets word and character spacing from its first
                // two operands, then shows its third.
                "Tj" | "TJ" | "'" | "\"" => {
                    if op.operator == "'" {
                        cursor.next_line_offset(0.0, -state.leading);
                    } else if op.operator == "\"" {
                        state.word_spacing = operand_f64(&op.operands, 0);
                        state.char_spacing = operand_f64(&op.operands, 1);
                        cursor.next_line_offset(0.0, -state.leading);
                    }
                    let Some(selected) = font else {
                        return Err(Error::redaction(format!(
                            "page {} draws text before selecting a font; \
                             its text cannot be decoded for redaction",
                            self.page
                        )));
                    };
                    let Some(text) = selected.text else {
                        return Err(Error::redaction(format!(
                            "page {} draws text with a font whose encoding \
                             could not be decoded or was not found in the page \
                             resources; its text cannot be redacted",
                            self.page
                        )));
                    };
                    let (operands, base): (&[Value], usize) = if op.operator == "\"" {
                        match op.operands.get(2) {
                            Some(s) => (slice::from_ref(s), 2),
                            None => (&[], 0),
                        }
                    } else {
                        (&op.operands, 0)
                    };
                    let shown = self.accum.rects.len();
                    // A short-lived run so its borrow of `accum` ends before
                    // the `Do` arm needs `accum` again.
                    let mut run = TextRun {
                        text: self.accum.text,
                        runs: self.accum.runs,
                        next_char: self.accum.next_char,
                        rects: self.accum.rects,
                        cursor: &mut cursor,
                        pen: &mut self.pen,
                        state,
                        widths: selected.widths,
                        metrics: selected.metrics,
                        stream,
                        font: selected.slot,
                    };
                    run.show(text, operands, op_idx, base)?;
                    // Text drawn with a pattern paints it where it shows,
                    // its stroke reaching past glyphs the clip hides.
                    let extent = self.accum.rects[shown..]
                        .iter()
                        .map(|glyph| glyph.rect)
                        .reduce(DeviceRect::union);
                    // A glyph shows only where the clip lets it, a form's or
                    // a pattern cell's `/BBox` included; one it hides has no
                    // box, though its text is still read.
                    let shown_glyphs: Vec<GlyphRect> = self
                        .accum
                        .rects
                        .drain(shown..)
                        .filter_map(|glyph| {
                            Some(GlyphRect {
                                rect: painting.visible(glyph.rect)?,
                                ..glyph
                            })
                        })
                        .collect();
                    self.accum.rects.extend(shown_glyphs);
                    if let Some(extent) = extent {
                        let painted = painting.text(extent, state.ctm, selected.outlines);
                        self.paint(painted);
                    }
                }

                // A `Do` may draw a Form XObject, whose content draws text in
                // its own stream and resources: walk it under the current
                // graphics state, the selected font and the text state, onto
                // whose CTM `walk_xobject` composes the form's `/Matrix`.
                "Do" => {
                    if let Some(Value::Name(name)) = op.operands.first() {
                        let inherited = Inherited {
                            font,
                            state,
                            painting,
                        };
                        self.walk_xobject(name, scope, depth, inherited)?;
                    }
                }

                // A tiling pattern selected to paint with draws its cell's
                // content, text included, wherever it paints. Selecting it
                // paints nothing, so its text is read once the page's is, never
                // inside it; any other colour paints with no pattern.
                "scn" | "SCN" => {
                    let pattern = match op.operands.last() {
                        Some(Value::Name(name)) => self.select_pattern(name, scope, depth),
                        _ => None,
                    };
                    let paint = pattern.map(|pattern| Paint {
                        pattern,
                        space: pattern_space,
                    });
                    if op.operator == "scn" {
                        painting.fill = paint;
                    } else {
                        painting.stroke = paint;
                    }
                }
                "gs" => {
                    if let Some(Value::Name(name)) = op.operands.first()
                        && let Some(graphics) = scope
                            .lookup(b"ExtGState", name)
                            .and_then(|graphics| graphics.value.into_dict())
                    {
                        painting.state(&graphics);
                    }
                }
                "sc" | "g" | "rg" | "k" | "cs" => painting.fill = None,
                "SC" | "G" | "RG" | "K" | "CS" => painting.stroke = None,
                _ => {}
            }
        }
        Ok(())
    }
}
