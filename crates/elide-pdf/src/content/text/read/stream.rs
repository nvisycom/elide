//! One content stream walked in operator order: the graphics-state stack,
//! text positioning, and the operators that show text or draw a form or
//! pattern.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::{array, mem, slice};

use elide_core::{Error, Result};
use lopdf::Object;
use lopdf::content::{Content, Operation};

use super::geometry::{Cursor, TextState};
use super::paint::{Paint, Painting};
use super::{TextRun, Walk};
use crate::content::text::{ClippingText, FontEntry, GlyphBytes, Mover, StreamTarget};
use crate::pdf::reader::font::{FontText, GlyphMetrics, WidthSource};
use crate::pdf::reader::{DeviceRect, DictionaryExt, DocumentExt, Matrix, ObjectExt, Scope};

/// The numeric operand at `index` as `f64`, or `0.0` when absent or non-numeric
/// (a malformed operator reads as its zero, never a hard error — positioning is
/// best-effort for spacing).
pub(super) fn operand_f64(operands: &[Object], index: usize) -> f64 {
    operands
        .get(index)
        .map_or(0.0, |o| o.as_number().unwrap_or(0.0))
}

/// Whether the text-showing `operands` draw a glyph: a string of them, or
/// one in a `TJ` array, holds a byte.
fn draws_glyphs(operands: &[Object]) -> bool {
    operands.iter().any(|operand| match operand {
        Object::String(bytes, _) => !bytes.is_empty(),
        Object::Array(items) => items
            .iter()
            .any(|item| matches!(item, Object::String(bytes, _) if !bytes.is_empty())),
        _ => false,
    })
}

/// Read six numeric operands from `base` as a [`Matrix`] `[a b c d e f]`, or
/// `None` when fewer than six are present.
pub(super) fn read_matrix(operands: &[Object], base: usize) -> Option<Matrix> {
    (operands.len() >= base + 6)
        .then(|| Matrix::from(array::from_fn(|i| operand_f64(operands, base + i))))
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
    by_name: BTreeMap<Vec<u8>, Rc<ScopeFont>>,
}

/// One font a scope names, read.
pub(super) struct ScopeFont {
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

/// The graphics state text is drawn under (8.4): the selected font, the
/// parametric text state, and what it paints with. `q` saves it and `Q`
/// restores it; the text matrices are not part of it, as they reset at `BT`.
/// A `Do`-invoked Form XObject starts from its caller's, with the form's
/// `/Matrix` composed onto the CTM, so the form's text is decoded with that
/// font before its own `Tf`, positioned in the page's space, and a fill it
/// makes before its own colour paints the caller's pattern.
#[derive(Clone, Copy, Default)]
pub(super) struct Graphics<'a> {
    /// The selected font, or `None` before any is.
    pub(super) font: Option<Font<'a>>,
    /// The parametric text state, the CTM included.
    pub(super) state: TextState,
    /// The patterns, clip and line style.
    pub(super) painting: Painting,
}

/// What a content stream is walked with: the stream its glyphs are stamped
/// with, so a deletion edits the right one; the scope its names resolve in;
/// the nesting depth left; and the caller's graphics state at the `Do` that
/// drew it (`None` at page level, where the state starts fresh).
pub(super) struct Frame<'s, 'i, 'd> {
    pub(super) stream: StreamTarget,
    pub(super) scope: &'s Scope<'d>,
    pub(super) depth: u8,
    pub(super) inherited: Option<Graphics<'i>>,
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
    ///
    /// A font held as an object of its own is read once a page, however many
    /// streams' scopes name it, and keeps one slot.
    fn fonts(&mut self, scope: &Scope<'d>) -> Result<Fonts> {
        let doc = self.doc;
        let mut by_name = BTreeMap::new();
        for (name, entry) in scope.entries(doc, b"Font") {
            let Some(font) = doc.resolve_dict(entry) else {
                continue;
            };
            let id = entry.as_reference().ok();
            if let Some(read) = id.and_then(|id| self.read_fonts.get(&id)) {
                by_name.insert(name.to_vec(), Rc::clone(read));
                continue;
            }
            let to_unicode = font
                .get(b"ToUnicode")
                .ok()
                .and_then(|o| o.as_reference().ok());
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
            let read = Rc::new(ScopeFont {
                slot,
                text: FontText::of(doc, font)?,
                widths: WidthSource::from_font(doc, font),
                metrics: GlyphMetrics::from_font(doc, font),
                outlines: !font.is_subtype(doc, b"Type3"),
            });
            if let Some(id) = id {
                self.read_fonts.insert(id, Rc::clone(&read));
            }
            by_name.insert(name.to_vec(), read);
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
        content: &Content,
        frame: Frame<'_, '_, 'd>,
    ) -> Result<()> {
        let Frame {
            stream,
            scope,
            depth,
            inherited,
        } = frame;
        let fonts = self.fonts(scope)?;
        // The graphics state: the caller's at a `Do` (its CTM already carries
        // the form's `/Matrix`, and its path stays behind), else a fresh one.
        let mut graphics = inherited.map_or_else(Graphics::default, |caller| Graphics {
            painting: caller.painting.inherited(),
            ..caller
        });
        let mut saved: Vec<Graphics<'_>> = Vec::new();
        // The text matrices, reset at `BT`. The pen, where the last glyph left
        // off, is shared across this stream and every form it draws, so a word
        // gap is judged consistently across a `Do` boundary.
        let mut cursor = Cursor::new(self.accum.new_line());
        // The glyphs the open text object shows in a clipping rendering mode.
        let mut clipping: Vec<GlyphBytes> = Vec::new();
        // The space a pattern this stream selects is laid out in (8.7.3.1).
        let pattern_space = graphics.state.ctm;

        for (op_idx, op) in content.operations.iter().enumerate() {
            if let Some(painted) = graphics.painting.path(op, graphics.state.ctm) {
                self.paint(painted);
                continue;
            }
            match op.operator.as_str() {
                // Graphics-state save/restore. A `Q` with an empty stack is a
                // malformed balance we ignore.
                "q" => saved.push(graphics),
                "Q" => {
                    if let Some(restored) = saved.pop() {
                        graphics.state = restored.state;
                        graphics.painting.restore(restored.painting);
                        graphics.font = restored.font;
                    }
                }
                "cm" => {
                    if let Some(m) = read_matrix(&op.operands, 0) {
                        graphics.state.ctm = m.then(graphics.state.ctm);
                    }
                }

                // Text object: the text and line matrices reset to the
                // identity (9.4.1). The pen is kept across the
                // boundary: it is a device-space position, unaffected by the
                // matrix reset, so a gap between one text object's last glyph
                // and the next object's first is judged by the same rule. Many
                // producers emit one `BT`/`ET` per positioned field, so nulling
                // the pen here would run every field into the next.
                "BT" => {
                    cursor = Cursor::new(self.accum.new_line());
                    graphics.painting.begin_text();
                    clipping.clear();
                }
                "ET" => {
                    graphics.painting.end_text();
                    if !clipping.is_empty() {
                        self.accum.clipping.push(ClippingText {
                            stream,
                            end: op_idx,
                            glyphs: mem::take(&mut clipping),
                        });
                    }
                }

                // Text state parameters.
                "Tc" => graphics.state.char_spacing = operand_f64(&op.operands, 0),
                "Tw" => graphics.state.word_spacing = operand_f64(&op.operands, 0),
                "Tz" => graphics.state.h_scale = operand_f64(&op.operands, 0) / 100.0,
                "TL" => graphics.state.leading = operand_f64(&op.operands, 0),
                "Ts" => graphics.state.rise = operand_f64(&op.operands, 0),
                "Tf" => {
                    if let Some(Object::Name(name)) = op.operands.first() {
                        graphics.font = Some(fonts.select(name, graphics.font));
                    }
                    graphics.state.font_size = operand_f64(&op.operands, 1);
                }

                // Text positioning: move the line/text matrix.
                "Td" => cursor.next_line_offset(
                    operand_f64(&op.operands, 0),
                    operand_f64(&op.operands, 1),
                    self.accum.new_line(),
                ),
                "TD" => {
                    // `TD tx ty` also sets leading to `-ty`.
                    let ty = operand_f64(&op.operands, 1);
                    graphics.state.leading = -ty;
                    cursor.next_line_offset(
                        operand_f64(&op.operands, 0),
                        ty,
                        self.accum.new_line(),
                    );
                }
                "Tm" => {
                    if let Some(m) = read_matrix(&op.operands, 0) {
                        cursor.set_matrix(m, self.accum.new_line());
                    }
                }
                "T*" => {
                    cursor.next_line_offset(0.0, -graphics.state.leading, self.accum.new_line());
                }

                // Text showing. `'` is an implicit `T*` then show; `"`
                // additionally sets word and character spacing from its first
                // two operands, then shows its third.
                "Tj" | "TJ" | "'" | "\"" => {
                    let from = self.accum.steps.len();
                    self.show_text(op, op_idx, &mut graphics, &mut cursor, stream)?;
                    if graphics.painting.clips_text()
                        && graphics.font.is_some_and(|font| font.outlines)
                    {
                        clipping.extend(self.accum.steps[from..].iter().filter_map(|step| {
                            match &step.mover {
                                Mover::Glyph { glyph, .. } => Some(glyph.clone()),
                                Mover::Kern(_) => None,
                            }
                        }));
                    }
                }

                // A `Do` may draw a Form XObject, whose content draws text in
                // its own stream and resources: walk it under the current
                // graphics state, onto whose CTM `walk_xobject` composes the
                // form's `/Matrix`.
                "Do" => {
                    if let Some(Object::Name(name)) = op.operands.first() {
                        self.walk_xobject(name, scope, depth, graphics)?;
                    }
                }

                // A tiling pattern selected to paint with draws its cell's
                // content, text included, wherever it paints. Selecting it
                // paints nothing, so its text is read once the page's is, never
                // inside it; any other colour paints with no pattern.
                "scn" | "SCN" => {
                    let pattern = match op.operands.last() {
                        Some(Object::Name(name)) => self.select_pattern(name, scope, depth),
                        _ => None,
                    };
                    let paint = pattern.map(|pattern| Paint {
                        pattern,
                        space: pattern_space,
                    });
                    if op.operator == "scn" {
                        graphics.painting.fill = paint;
                    } else {
                        graphics.painting.stroke = paint;
                    }
                }
                "gs" => {
                    if let Some(Object::Name(name)) = op.operands.first()
                        && let Some(parameters) = scope
                            .lookup(self.doc, b"ExtGState", name)
                            .and_then(|parameters| self.doc.resolve_dict(parameters))
                    {
                        graphics.painting.state(self.doc, parameters);
                        self.select_soft_mask(parameters, graphics.state.ctm, depth);
                    }
                }
                "sc" | "g" | "rg" | "k" | "cs" => graphics.painting.fill = None,
                "SC" | "G" | "RG" | "K" | "CS" => graphics.painting.stroke = None,
                _ => {}
            }
        }
        Ok(())
    }

    /// Show the text the text-showing operation `operation`, the `index`th of
    /// its stream, draws in the font `graphics` selects: `'` moves to the
    /// next line first, and `"` also sets word and character spacing from its
    /// first two operands before showing its third. Its glyphs are placed
    /// from `cursor` under the text state, kept only where the clip shows
    /// them, and painted with the patterns.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if no font
    /// is selected or its codes can't be read; as [`TextRun::show`].
    fn show_text(
        &mut self,
        operation: &Operation,
        index: usize,
        graphics: &mut Graphics<'_>,
        cursor: &mut Cursor,
        stream: StreamTarget,
    ) -> Result<()> {
        if operation.operator == "'" {
            cursor.next_line_offset(0.0, -graphics.state.leading, self.accum.new_line());
        } else if operation.operator == "\"" {
            graphics.state.word_spacing = operand_f64(&operation.operands, 0);
            graphics.state.char_spacing = operand_f64(&operation.operands, 1);
            cursor.next_line_offset(0.0, -graphics.state.leading, self.accum.new_line());
        }
        let Some(selected) = graphics.font else {
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
        let (operands, base) = shown_operands(operation);
        // Vertical writing is not laid out, so a page drawing it is
        // redacted from its render, and its text, which may map to
        // nothing readable, is not read.
        if text.is_vertical() {
            self.unplaced |= draws_glyphs(operands);
            return Ok(());
        }
        let shown = self.accum.rects.len();
        let mut run = TextRun {
            text: self.accum.text,
            runs: self.accum.runs,
            next_char: self.accum.next_char,
            rects: self.accum.rects,
            steps: self.accum.steps,
            cursor,
            pen: &mut self.pen,
            state: graphics.state,
            widths: selected.widths,
            metrics: selected.metrics,
            stream,
            font: selected.slot,
        };
        run.show(text, operands, index, base)?;
        if self.accum.rects.len() > shown && matches!(selected.widths, WidthSource::Unknown) {
            self.unplaced = true;
        }
        self.clip_shown(shown, graphics, selected.outlines);
        Ok(())
    }

    /// Give the glyphs placed from the `from`th on `graphics`' clip, a form's
    /// or a pattern cell's `/BBox` included: one it hides shows no box,
    /// though its text is still read. Text drawn with a pattern
    /// paints it where the glyphs are, its stroke reaching past those the
    /// clip hides, so their extent is painted first; `outlines` says whether
    /// a clipping rendering mode clips to them.
    fn clip_shown(&mut self, from: usize, graphics: &mut Graphics<'_>, outlines: bool) {
        let extent = self.accum.rects[from..]
            .iter()
            .map(|glyph| glyph.extent)
            .reduce(DeviceRect::union);
        let clip = graphics.painting.clip();
        for glyph in &mut self.accum.rects[from..] {
            glyph.clip = clip;
        }
        if let Some(extent) = extent {
            let painted = graphics.painting.text(extent, graphics.state.ctm, outlines);
            self.paint(painted);
        }
    }
}

/// The operands of the text-showing `operation` that show text, and the
/// index of the first among all its operands: `"`'s third, after the word
/// and character spacing it sets; every other operator's all.
fn shown_operands(operation: &Operation) -> (&[Object], usize) {
    if operation.operator != "\"" {
        return (&operation.operands, 0);
    }
    match operation.operands.get(2) {
        Some(shown) => (slice::from_ref(shown), 2),
        None => (&[], 0),
    }
}
