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
//! The vocabulary follows the PDF specification (ISO 32000-1 §9.4): a [`Matrix`]
//! is the 2×3 affine `[a b c d e f]`; [`TextState`] is the parametric text state
//! the `q`/`Q` graphics-state stack saves; the text matrix and line matrix are
//! *not* part of that state (they reset at `BT` and are set by the text-
//! positioning operators), so [`Cursor`] holds them separately.

use elide_core::primitive::UnitBoundingBox;

/// A 2×3 affine transform `[a b c d e f]`, mapping a point `(x, y)` to
/// `(a·x + c·y + e, b·x + d·y + f)`. Row-vector convention, as PDF uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Matrix {
    pub(super) a: f64,
    pub(super) b: f64,
    pub(super) c: f64,
    pub(super) d: f64,
    pub(super) e: f64,
    pub(super) f: f64,
}

impl Matrix {
    /// The identity transform.
    pub(super) const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A pure translation by `(tx, ty)`.
    pub(super) fn translation(tx: f64, ty: f64) -> Matrix {
        Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// `self × other` (apply `self` first, then `other`), the composition PDF text
    /// positioning uses for `Tlm ← translate · Tlm`.
    pub(super) fn then(self, other: Matrix) -> Matrix {
        Matrix {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    /// The device-space origin `(e, f)` — where this matrix places `(0, 0)`.
    pub(super) fn origin(self) -> (f64, f64) {
        (self.e, self.f)
    }
}

/// The parametric text state (ISO 32000-1 §9.3) the `q`/`Q` stack saves and
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
    /// Text rise `Ts` (text-space units). Tracked for completeness; horizontal
    /// spacing does not use it.
    pub(super) rise: f64,
    /// The current transformation matrix (from `cm`), mapping text/user space to
    /// device space. Combined with the text matrix to place a glyph on the page.
    pub(super) ctm: Matrix,
}

impl Default for TextState {
    fn default() -> Self {
        TextState {
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

/// The text matrix `Tm` and text line matrix `Tlm` (ISO 32000-1 §9.4.2). Both
/// reset to the identity at `BT`; the positioning operators move them, and each
/// shown glyph advances `Tm`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Cursor {
    tm: Matrix,
    tlm: Matrix,
}

impl Cursor {
    /// A cursor at the text-object origin, as `BT` establishes.
    pub(super) fn new() -> Cursor {
        Cursor {
            tm: Matrix::IDENTITY,
            tlm: Matrix::IDENTITY,
        }
    }

    /// `Tm a b c d e f`: set both the text and line matrix absolutely.
    pub(super) fn set_matrix(&mut self, m: Matrix) {
        self.tm = m;
        self.tlm = m;
    }

    /// `Td tx ty`: move to the start of the next line, offset by `(tx, ty)` from
    /// the current line's start.
    pub(super) fn next_line_offset(&mut self, tx: f64, ty: f64) {
        self.tlm = Matrix::translation(tx, ty).then(self.tlm);
        self.tm = self.tlm;
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
}

/// An axis-aligned box in device (page) space, as the glyph quad projects to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DeviceRect {
    pub(super) min_x: f64,
    pub(super) min_y: f64,
    pub(super) max_x: f64,
    pub(super) max_y: f64,
}

impl DeviceRect {
    /// The axis-aligned bounds of four device-space corners (a glyph quad mapped
    /// through a possibly-rotated matrix is not axis-aligned; take its extent).
    pub(super) fn of_corners(corners: [(f64, f64); 4]) -> DeviceRect {
        let xs = corners.map(|(x, _)| x);
        let ys = corners.map(|(_, y)| y);
        DeviceRect {
            min_x: xs.iter().copied().fold(f64::INFINITY, f64::min),
            min_y: ys.iter().copied().fold(f64::INFINITY, f64::min),
            max_x: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            max_y: ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
    }

    /// The union covering both boxes, for merging a line's glyphs into one rect.
    pub(super) fn union(self, other: DeviceRect) -> DeviceRect {
        DeviceRect {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }
}

/// The visible page rectangle and its display rotation, for normalizing a
/// device-space [`DeviceRect`] into a page-normalized [`UnitBoundingBox`].
///
/// The walker's device coordinates are in unrotated PDF user space (bottom-left
/// origin). A viewer displays the page's [`CropBox`](Self::crop) region, rotated
/// by [`rotate`](Self::rotate) degrees, so normalization must (1) place the crop
/// origin at zero, (2) divide by the crop size, (3) apply the rotation, and
/// (4) flip Y to the top-left origin a renderer uses.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageFrame {
    /// CropBox lower-left corner in user space (fallback: MediaBox).
    pub(super) crop_x0: f64,
    pub(super) crop_y0: f64,
    /// CropBox width and height in user space (always positive).
    pub(super) crop_w: f64,
    pub(super) crop_h: f64,
    /// Display rotation, normalized to one of 0, 90, 180, 270.
    pub(super) rotate: u32,
}

impl PageFrame {
    /// Normalize a device-space box into a page-normalized, top-left-origin
    /// [`UnitBoundingBox`] in the as-displayed (rotated) frame.
    pub(super) fn normalize(&self, rect: DeviceRect) -> UnitBoundingBox {
        // Guard degenerate crop dimensions so normalization never divides by zero.
        let cw = self.crop_w.max(f64::MIN_POSITIVE);
        let ch = self.crop_h.max(f64::MIN_POSITIVE);

        // Step 1+2: crop-relative fractions, still bottom-left origin, unrotated.
        let fx0 = (rect.min_x - self.crop_x0) / cw;
        let fx1 = (rect.max_x - self.crop_x0) / cw;
        let fy0 = (rect.min_y - self.crop_y0) / ch;
        let fy1 = (rect.max_y - self.crop_y0) / ch;

        // Step 3+4: rotate the two corners into the displayed frame, then flip Y
        // to a top-left origin. Rotation maps the unrotated unit square to the
        // displayed one; we rotate both corners and re-derive the min/extent.
        let p0 = rotate_unit_point(fx0, fy0, self.rotate);
        let p1 = rotate_unit_point(fx1, fy1, self.rotate);
        let (min_x, max_x) = (p0.0.min(p1.0), p0.0.max(p1.0));
        // Bottom-left-origin y in the rotated frame → top-left by `1 - y`.
        let top0 = 1.0 - p0.1;
        let top1 = 1.0 - p1.1;
        let (min_y, max_y) = (top0.min(top1), top0.max(top1));

        UnitBoundingBox::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }
}

/// Rotate a point in the unrotated unit square (bottom-left origin) into the
/// displayed frame for a page `/Rotate` of 0/90/180/270 degrees clockwise.
fn rotate_unit_point(x: f64, y: f64, rotate: u32) -> (f64, f64) {
    match rotate {
        90 => (y, 1.0 - x),
        180 => (1.0 - x, 1.0 - y),
        270 => (1.0 - y, x),
        _ => (x, y),
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

    /// Whether a space belongs before a glyph that starts at device x `next_x` on
    /// baseline `next_y`, given the previous glyph's advance ended the pen at
    /// `pen_x` on baseline `pen_y`, with scaled font size `em` in device units.
    ///
    /// A baseline move is always a boundary. Otherwise a forward gap of at least
    /// [`GAP_EM`](Self::GAP_EM) of `em` is a space; a backward or tiny move
    /// (kerning, overlap) is not.
    pub(super) fn needs_space(pen_x: f64, pen_y: f64, next_x: f64, next_y: f64, em: f64) -> bool {
        // A changed baseline is a new line: a word boundary regardless of x.
        if (next_y - pen_y).abs() > em * Self::GAP_EM {
            return true;
        }
        next_x - pen_x >= em * Self::GAP_EM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// An unrotated A4-ish page: a device box maps to crop fractions with the Y
    /// axis flipped to a top-left origin.
    #[test]
    fn normalize_unrotated_flips_y() {
        let frame = PageFrame {
            crop_x0: 0.0,
            crop_y0: 0.0,
            crop_w: 100.0,
            crop_h: 200.0,
            rotate: 0,
        };
        // A box near the page bottom-left in PDF space (y up).
        let rect = DeviceRect {
            min_x: 10.0,
            min_y: 20.0,
            max_x: 30.0,
            max_y: 60.0,
        };
        let u = frame.normalize(rect);
        // x: 10/100..30/100. y (top-left): top = 1 - 60/200 = 0.7, height = 40/200.
        assert!(close(u.x, 0.1), "{u:?}");
        assert!(close(u.width, 0.2), "{u:?}");
        assert!(close(u.y, 0.7), "{u:?}");
        assert!(close(u.height, 0.2), "{u:?}");
    }

    /// A non-zero crop origin shifts the box before normalizing.
    #[test]
    fn normalize_respects_crop_origin() {
        let frame = PageFrame {
            crop_x0: 50.0,
            crop_y0: 100.0,
            crop_w: 100.0,
            crop_h: 100.0,
            rotate: 0,
        };
        let rect = DeviceRect {
            min_x: 60.0,
            min_y: 110.0,
            max_x: 70.0,
            max_y: 120.0,
        };
        let u = frame.normalize(rect);
        // x: (60-50)/100 = 0.1. top: 1 - (120-100)/100 = 0.8.
        assert!(close(u.x, 0.1), "{u:?}");
        assert!(close(u.width, 0.1), "{u:?}");
        assert!(close(u.y, 0.8), "{u:?}");
        assert!(close(u.height, 0.1), "{u:?}");
    }

    /// A 90° page rotation maps the unrotated unit square into the displayed
    /// frame: a point at unrotated (x, y) lands at (y, 1 - x) before the Y-flip.
    #[test]
    fn normalize_rotates_90() {
        let frame = PageFrame {
            crop_x0: 0.0,
            crop_y0: 0.0,
            crop_w: 100.0,
            crop_h: 100.0,
            rotate: 90,
        };
        // Unrotated crop fractions: x 0.1..0.2, y 0.3..0.4 (bottom-left origin).
        let rect = DeviceRect {
            min_x: 10.0,
            min_y: 30.0,
            max_x: 20.0,
            max_y: 40.0,
        };
        let u = frame.normalize(rect);
        // rotate_unit_point(x,y,90) = (y, 1-x). Corners (0.1,0.3)->(0.3,0.9),
        // (0.2,0.4)->(0.4,0.8). x: 0.3..0.4. bottom-left y: 0.8..0.9 → top-left
        // (1-y): 0.1..0.2.
        assert!(close(u.x, 0.3), "{u:?}");
        assert!(close(u.width, 0.1), "{u:?}");
        assert!(close(u.y, 0.1), "{u:?}");
        assert!(close(u.height, 0.1), "{u:?}");
    }
}
