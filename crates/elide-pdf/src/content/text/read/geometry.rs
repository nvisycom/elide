//! Text-space geometry: the matrices and text state a content walk tracks to
//! reconstruct where each glyph lands on the page, so inter-word spacing can be
//! recovered.
//!
//! A PDF content stream does not store spaces between words — it positions glyphs
//! with matrices and advances. Word gaps are *implicit* in those positions: a
//! producer may draw `(Hello) Tj` then jump the pen and draw `(world) Tj` with no
//! space character between them. To recover the logical text a caller runs
//! detection over, the walk replays the same positioning the viewer does: it
//! tracks the text matrix, advances the pen by each glyph's width, and — when the
//! next glyph starts further along than its predecessor's advance accounts for —
//! reads the gap as a space.
//!
//! The vocabulary follows the PDF specification (9.4): a
//! [`Matrix`] places text space on the page; [`TextState`] is the parametric text state
//! the `q`/`Q` graphics-state stack saves; the text matrix and line matrix are
//! *not* part of that state (they reset at `BT` and are set by the text-
//! positioning operators), so [`Cursor`] holds them separately.

use crate::pdf::reader::Matrix;

/// The parametric text state (9.3) the `q`/`Q` stack saves and
/// restores. The text matrix and line matrix are deliberately absent: the spec
/// makes them reset at `BT`, not part of the saved graphics state.
#[derive(Debug, Clone, Copy)]
pub(super) struct TextState {
    /// Font size set by `Tf` (text-space units). Zero until the first `Tf`.
    pub(super) font_size: f64,
    /// Character spacing `Tc` (unscaled text-space units), added after each glyph.
    pub(super) char_spacing: f64,
    /// Word spacing `Tw` (unscaled text-space units), added after each single-byte
    /// space (code 32) — never for multi-byte composite-font codes, per spec.
    pub(super) word_spacing: f64,
    /// Horizontal scaling `Tz`, as a fraction (`Tz / 100`).
    pub(super) h_scale: f64,
    /// Text leading `TL` (text-space units): the line-to-line advance `T*` uses.
    pub(super) leading: f64,
    /// Text rise `Ts` (text-space units): how far glyphs are drawn above the
    /// baseline, which raises their boxes but not the pen.
    pub(super) rise: f64,
    /// The current transformation matrix (from `cm`), mapping text/user space to
    /// device space. Combined with the text matrix to place a glyph on the page.
    pub(super) ctm: Matrix,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            font_size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            ctm: Matrix::IDENTITY,
        }
    }
}

/// The text matrix `Tm` and text line matrix `Tlm` (9.4.2). Both
/// reset to the identity at `BT`; the positioning operators move them, and each
/// shown glyph advances `Tm`. Each setting of the line matrix starts a line,
/// numbered by the caller.
#[derive(Debug, Clone, Copy)]
pub(super) struct Cursor {
    tm: Matrix,
    tlm: Matrix,
    line: u64,
}

impl Cursor {
    /// A cursor at the text-object origin, as `BT` establishes, starting line
    /// `line`.
    pub(super) fn new(line: u64) -> Self {
        Self {
            tm: Matrix::IDENTITY,
            tlm: Matrix::IDENTITY,
            line,
        }
    }

    /// The line the cursor places glyphs on.
    pub(super) fn line(&self) -> u64 {
        self.line
    }

    /// `Tm a b c d e f`: set both the text and line matrix absolutely,
    /// starting line `line`.
    pub(super) fn set_matrix(&mut self, m: Matrix, line: u64) {
        self.tm = m;
        self.tlm = m;
        self.line = line;
    }

    /// `Td tx ty`: move to the start of the next line, offset by `(tx, ty)` from
    /// the current line's start, starting line `line`.
    pub(super) fn next_line_offset(&mut self, tx: f64, ty: f64, line: u64) {
        self.tlm = Matrix::translation(tx, ty).then(self.tlm);
        self.tm = self.tlm;
        self.line = line;
    }

    /// Advance the text matrix by a horizontal displacement `tx` (text-space
    /// units, already scaled), as a shown glyph or a `TJ` kern does.
    pub(super) fn advance(&mut self, tx: f64) {
        self.tm = Matrix::translation(tx, 0.0).then(self.tm);
    }

    /// The glyph origin in device space: the text matrix combined with the CTM,
    /// origin extracted. This is where the next glyph would be drawn — its x locates
    /// a horizontal gap, its y a line change.
    pub(super) fn device_origin(&self, ctm: Matrix) -> (f64, f64) {
        self.tm.then(ctm).origin()
    }

    /// Map a point given in *text space* (relative to the current text origin)
    /// through the text matrix and CTM to device space. Used to project a
    /// glyph's box corners (advance × ascent/descent) onto the page.
    pub(super) fn device_point(&self, ctm: Matrix, tx: f64, ty: f64) -> (f64, f64) {
        let m = self.tm.then(ctm);
        (m.a * tx + m.c * ty + m.e, m.b * tx + m.d * ty + m.f)
    }

    /// The writing-direction unit vector in device space and its scale: the
    /// x-basis of the text-to-device transform (`tm·ctm`), normalized, plus its
    /// pre-normalized length (how much one text-space unit scales on the page).
    /// This is the direction a glyph advance moves the pen, so comparing device
    /// deltas along it handles rotated/sheared text, not just upright. A
    /// degenerate basis falls back to the x-axis so a caller never divides by
    /// zero.
    pub(super) fn writing_axis(&self, ctm: Matrix) -> ((f64, f64), f64) {
        let m = self.tm.then(ctm);
        let len = m.a.hypot(m.b);
        if len <= f64::MIN_POSITIVE {
            ((1.0, 0.0), 0.0)
        } else {
            ((m.a / len, m.b / len), len)
        }
    }
}

/// Decides, from realized geometry, whether a synthetic space belongs between two
/// consecutive shown glyphs.
///
/// The rule is the one a faithful extractor uses: compare where the next glyph
/// actually starts to where the previous glyph's advance left the pen. A forward
/// jump wider than a fraction of the font size is a word gap; a baseline change
/// is a line break (also a word boundary). The fraction is deliberately generous
/// — a true word space advances by the space glyph's width (commonly ~0.25 em),
/// and intra-word kerning is far smaller — so ordinary kerning never reads as a
/// space while a real gap always does.
pub(super) struct SpaceRule;

impl SpaceRule {
    /// The gap threshold as a fraction of the (scaled) font size. A forward
    /// horizontal gap at or above this reads as a word space.
    const GAP_EM: f64 = 0.2;

    /// Whether a space belongs before a glyph, from the device-space move from
    /// the previous glyph's pen (`pen`) to this glyph's origin (`next`), the
    /// writing-direction unit vector `axis` (device space), and the scaled font
    /// size `em` in device units.
    ///
    /// The move is split into its component *along* the writing direction
    /// (forward advance) and *perpendicular* to it (a baseline change). A
    /// perpendicular shift beyond [`GAP_EM`](Self::GAP_EM) of `em` is a new line
    /// (a word boundary); otherwise a forward advance of at least that threshold
    /// is a word space. Projecting onto the axis — rather than comparing raw x/y
    /// — makes this correct for rotated and vertical text, not just upright.
    pub(super) fn needs_space(
        pen: (f64, f64),
        next: (f64, f64),
        axis: (f64, f64),
        em: f64,
    ) -> bool {
        let dx = next.0 - pen.0;
        let dy = next.1 - pen.1;
        // Component along the writing direction, and perpendicular to it.
        let along = dx * axis.0 + dy * axis.1;
        let perp = dx * -axis.1 + dy * axis.0;
        let threshold = em * Self::GAP_EM;
        if perp.abs() > threshold {
            return true;
        }
        along >= threshold
    }
}
