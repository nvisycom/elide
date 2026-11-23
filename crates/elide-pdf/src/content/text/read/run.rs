//! A page's text and glyph map as the walk shows glyphs: each glyph's
//! characters and box, and the spaces positioning implies.

use std::ops::Range;

use elide_core::Result;
use lopdf::Object;

use super::geometry::{Cursor, SpaceRule, TextState};
use crate::content::text::{
    Address, GlyphBytes, GlyphSite, Mover, OffsetRun, PenStep, StreamTarget,
};
use crate::pdf::reader::font::{FontText, Glyph, GlyphMetrics, WidthSource, decode_glyphs};
use crate::pdf::reader::{DeviceRect, GlyphRect, Matrix, ObjectExt, Point};

/// The character code of the single-byte space, the only code `Tw` word spacing
/// applies to (9.3.3).
pub(super) const SPACE_CODE: u32 = 32;

/// Append a synthetic space no glyph drew to `text`, its run to `runs`: one
/// char of gap with no glyph source (a redaction leaves it alone). Coalesces:
/// never two gaps in a row, and never a leading gap.
pub(super) fn gap(text: &mut String, next_char: &mut usize, runs: &mut Vec<OffsetRun>) {
    if text.is_empty() || text.ends_with(' ') {
        return;
    }
    let start = *next_char;
    text.push(' ');
    *next_char += 1;
    runs.push(OffsetRun {
        chars: start..*next_char,
        source: None,
    });
}

/// A page's text and glyph map under construction, driven by the text geometry.
///
/// [`show`](TextRun::show) decodes a `Tj`/`TJ` operand list, appending each
/// glyph's characters as one [`OffsetRun`] and advancing the pen by the glyph's
/// width; where a glyph lands beyond its predecessor's advance (a positional word
/// gap, or a `TJ` kern wide enough to be one) it inserts a synthetic space run
/// that no glyph drew. The same accumulation spans the page's own content and the
/// Form XObjects it draws, so a detected span is located wherever it was drawn,
/// and each glyph run records the stream its bytes live in.
pub(super) struct TextRun<'a> {
    pub(super) text: &'a mut String,
    pub(super) runs: &'a mut Vec<OffsetRun>,
    /// Next unconsumed decoded-character offset (kept so each run's `chars`
    /// range is filled without recounting `text`).
    pub(super) next_char: &'a mut usize,
    /// The text matrix/line matrix, advanced as glyphs are shown.
    pub(super) cursor: &'a mut Cursor,
    /// Where the pen ended after the last shown glyph, in device space, or `None`
    /// before any glyph: the reference the next glyph's position is compared
    /// against to decide a word space. Carried across show-ops so a gap between
    /// two separate `Tj`s is seen.
    pub(super) pen: &'a mut Option<Point>,
    /// Per-glyph device-space boxes (char range → box), parallel to `runs`, for
    /// building each entity's rendered geometry. A synthetic gap records none.
    pub(super) rects: &'a mut Vec<GlyphRect>,
    /// Every move of the pen, in draw order.
    pub(super) steps: &'a mut Vec<PenStep>,
    /// The current parametric text state (font size, spacings, scaling, CTM).
    pub(super) state: TextState,
    /// The selected font's width table, for the per-glyph advance.
    pub(super) widths: &'a WidthSource,
    /// The selected font's vertical metrics, for the per-glyph box height.
    pub(super) metrics: GlyphMetrics,
    /// The content stream currently being walked, stamped onto every glyph so a
    /// deletion edits the right stream.
    pub(super) stream: StreamTarget,
    /// The font-table index of the currently selected font, stamped onto every
    /// glyph pushed so a deletion can later reach that font's `/ToUnicode`.
    pub(super) font: u16,
}

impl TextRun<'_> {
    /// Append the characters `glyph` drew (its decoded text) as one run tied
    /// to its bytes at `address`.
    pub(super) fn push_glyph(&mut self, address: Address, glyph: &Glyph) {
        let start = *self.next_char;
        let count = glyph.text.chars().count();
        if count == 0 {
            return;
        }
        self.text.push_str(&glyph.text);
        *self.next_char += count;
        self.runs.push(OffsetRun {
            chars: start..*self.next_char,
            source: Some(GlyphBytes {
                address,
                byte_start: glyph.bytes.start,
                byte_end: glyph.bytes.end,
                code: glyph.code,
                font: self.font,
            }),
        });
    }

    /// Append a synthetic space that no glyph drew: one char of gap with no glyph
    /// source (a redaction leaves it alone). Coalesces: never two gaps in a row,
    /// and never a leading gap.
    pub(super) fn push_gap(&mut self) {
        gap(self.text, self.next_char, self.runs);
    }

    /// Draw a `Tj`/`TJ` operand list, decoding glyphs, recording their bytes, and
    /// advancing the pen so inter-glyph gaps become spaces.
    ///
    /// `operand_base` is the index of `operands[0]` within the operation's full
    /// operand list, so the recorded [`Address`] names the true operand even when
    /// the caller passes a sub-slice (as `"` does, whose string is operand 2). It
    /// is 0 when the whole operand list is passed.
    pub(super) fn show(
        &mut self,
        enc: &FontText,
        operands: &[Object],
        op: usize,
        operand_base: usize,
    ) -> Result<()> {
        let stream = self.stream;
        for (i, value) in operands.iter().enumerate() {
            let operand = operand_base + i;
            let address = |item| Address {
                stream,
                op,
                operand,
                item,
            };
            match value {
                Object::String(bytes, _) => self.show_string(enc, bytes, address(None))?,
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
                                self.show_string(enc, bytes, address(Some(item_idx)))?;
                            }
                            Object::Integer(_) | Object::Real(_) => {
                                let kern = item.as_number().unwrap_or(0.0);
                                // Displacement is −kern/1000 × font size × hscale.
                                let tx = -kern / 1000.0 * self.state.font_size * self.state.h_scale;
                                self.cursor.advance(tx);
                                self.step(Mover::Kern(address(Some(item_idx))), tx);
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
    pub(super) fn show_string(
        &mut self,
        enc: &FontText,
        bytes: &[u8],
        address: Address,
    ) -> Result<()> {
        for glyph in decode_glyphs(enc, bytes)? {
            let w0 = self.widths.advance(glyph.width_key).unwrap_or(0.0) / 1000.0;
            self.space_before(w0);
            let start = *self.next_char;
            self.push_glyph(address, &glyph);
            let chars = start..*self.next_char;
            let drawn = chars.clone();
            // The glyph's advance: its visible width only, no `Tc`/`Tw`. An
            // unknown width gives a zero-width advance; its box still has its
            // height.
            let advance = w0 * self.state.font_size * self.state.h_scale;
            if !chars.is_empty() {
                let rect = self.glyph_rect(chars, advance);
                self.rects.push(rect);
            }
            let word = self.word_spacing(&glyph);
            let number = self.number(w0, word);
            let tx = self.advance(advance, word);
            let site = GlyphSite {
                address,
                byte_start: glyph.bytes.start,
            };
            let mover = Mover::Glyph {
                site,
                chars: drawn,
                number,
            };
            self.step(mover, tx);
        }
        Ok(())
    }

    /// Record that `mover` moved the pen `advance` along the line in text
    /// space.
    fn step(&mut self, mover: Mover, advance: f64) {
        self.steps.push(PenStep {
            line: self.cursor.line(),
            mover,
            advance,
        });
    }

    /// The word spacing `glyph` moves the pen by: `Tw`, for a one-byte code
    /// 32 only.
    fn word_spacing(&self, glyph: &Glyph) -> f64 {
        if glyph.code == SPACE_CODE && glyph.bytes.len() == 1 {
            self.state.word_spacing
        } else {
            0.0
        }
    }

    /// The `TJ` number that moves the pen as a glyph `w0` em wide and spaced
    /// `word` beyond `Tc` does: `-(w0 × 1000 + 1000 × (Tc + word) / Tfs)`, a
    /// number's displacement being `-n / 1000 × Tfs × Th` and the glyph's
    /// `(w0 × Tfs + Tc + word) × Th`. No number does at font size 0.
    fn number(&self, w0: f64, word: f64) -> Option<f64> {
        let size = self.state.font_size;
        (size != 0.0).then(|| -(w0 * 1000.0 + 1000.0 * (self.state.char_spacing + word) / size))
    }

    /// Push a word space before a glyph `w0` em wide when it lands past the
    /// gap threshold from where the previous glyph left the pen; before the
    /// first glyph there is nothing to compare against. The threshold is a
    /// fraction of the font size in device units, measured along the
    /// advance, so it holds under rotation and shear. The advance runs
    /// backwards along the matrix's x-basis when `font_size · h_scale` is
    /// negative (mirrored text, a negative `Tz`) or the glyph's own advance
    /// is (a Type3 font whose `/FontMatrix` mirrors it), so the axis flips
    /// then, and `em` is kept positive for a degenerate transform.
    fn space_before(&mut self, w0: f64) {
        let Some(pen) = *self.pen else {
            return;
        };
        let origin = self.cursor.transform(self.state.ctm).origin();
        let (mut axis, scale) = self.cursor.writing_axis(self.state.ctm);
        let signed = self.state.font_size * self.state.h_scale;
        if signed * w0.signum() < 0.0 {
            axis = -axis;
        }
        let em = (signed.abs() * scale).max(f64::MIN_POSITIVE);
        if SpaceRule::needs_space(pen, origin, axis, em) {
            self.push_gap();
        }
    }

    /// The device-space box of the glyph drawing `chars`, `advance` wide, in
    /// its own space: text space raised by the text rise (9.4.4), so the glyph
    /// is drawn from that space's origin. The box spans from the origin to the
    /// advance and from the descent to the ascent there, its four corners
    /// projected through the text matrix and CTM and their extent taken, so
    /// rotation and shear are handled.
    fn glyph_rect(&self, chars: Range<usize>, advance: f64) -> GlyphRect {
        let transform =
            Matrix::translation(0.0, self.state.rise).then(self.cursor.transform(self.state.ctm));
        let extent = DeviceRect {
            min_x: 0.0,
            min_y: self.metrics.bottom * self.state.font_size,
            max_x: advance,
            max_y: self.metrics.top * self.state.font_size,
        };
        GlyphRect {
            chars,
            rect: extent.mapped(transform),
            transform,
        }
    }

    /// Move the pen past a glyph `advance` wide and spaced `word` beyond
    /// `Tc`: `advance + (Tc + word) × Th`. `Tc` and `Tw` space the pen beyond
    /// the glyph's own width, so they leave its box alone.
    fn advance(&mut self, advance: f64, word: f64) -> f64 {
        let tx = advance + (self.state.char_spacing + word) * self.state.h_scale;
        self.cursor.advance(tx);
        *self.pen = Some(self.cursor.transform(self.state.ctm).origin());
        tx
    }
}
