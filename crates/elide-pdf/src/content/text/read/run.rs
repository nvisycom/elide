//! A page's text and glyph map as the walk shows glyphs: each glyph's
//! characters and box, and the spaces positioning implies.

use elide_core::Result;
use lopdf::Object;

use super::geometry::{Cursor, SpaceRule, TextState};
use crate::content::text::{Address, GlyphBytes, OffsetRun, StreamTarget};
use crate::pdf::reader::font::{FontText, GlyphMetrics, WidthSource, decode_glyphs};
use crate::pdf::reader::{DeviceRect, GlyphRect, ObjectExt};

/// The character code of the single-byte space, the only code `Tw` word spacing
/// applies to (ISO 32000-1 §9.3.3).
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
    pub(super) pen: &'a mut Option<(f64, f64)>,
    /// Per-glyph device-space boxes (char range → box), parallel to `runs`, for
    /// building each entity's rendered geometry. A synthetic gap records none.
    pub(super) rects: &'a mut Vec<GlyphRect>,
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
    /// Append the characters a glyph drew (its decoded text) as one run tied to
    /// the glyph's bytes.
    pub(super) fn push_glyph(
        &mut self,
        text: &str,
        address: Address,
        byte_start: usize,
        byte_end: usize,
    ) {
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
    pub(super) fn show_string(
        &mut self,
        enc: &FontText<'_>,
        bytes: &[u8],
        address: Address,
    ) -> Result<()> {
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
