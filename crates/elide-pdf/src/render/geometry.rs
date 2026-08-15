//! Glyph geometry in rendered-pixel space: the bridge between detected text
//! spans and the pixels a raster redaction overwrites.
//!
//! Every geometry here is in the coordinate space of the *rendered page image*:
//! pixels, origin at the top-left, x rightward and y downward — the space the
//! raster fill works in. Text-layer glyphs (from the PDF's own text operators)
//! are converted into this space from PDF points; an OCR source (supplied by a
//! caller) is already here. Keeping one canonical space lets the fill treat
//! both sources identically.
//!
//! These types carry no elide dependency, so the crate stays standalone. They
//! are re-exported from [`render`](crate::render), so a caller reaches them as
//! `render::PixelRect` etc., not through this module path.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle in rendered-page pixels (top-left origin).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PixelRect {
    /// Left edge, in pixels from the page's left.
    pub x: u32,
    /// Top edge, in pixels from the page's top.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl PixelRect {
    /// The rectangle at `(x, y)` with the given size.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Convert a PDF-point box (bottom-left origin, as PDFium reports glyph
    /// bounds) into rendered-page pixels (top-left origin).
    ///
    /// `left`/`bottom`/`right`/`top` are the box edges in points; `page_height`
    /// is the page height in points; `scale_x`/`scale_y` map points to pixels.
    /// Edges round outward (`floor` the origin, `ceil` the size) so a redaction
    /// never leaves a sliver of the original glyph uncovered.
    pub(crate) fn from_points(
        left: f32,
        bottom: f32,
        right: f32,
        top: f32,
        page_height: f32,
        scale_x: f32,
        scale_y: f32,
    ) -> Self {
        // Y-flip: a point's distance from the page *bottom* becomes a pixel's
        // distance from the image *top*.
        //
        // Compute each scaled edge, then floor the origin and ceil the far edge
        // independently: the size is the ceiled far edge minus the floored
        // origin, so a box straddling pixel boundaries covers every pixel it
        // touches (e.g. x 10.5..11.4 covers pixels 10 and 11), not just the
        // ones a single ceiled width would span.
        let left_px = (left * scale_x).max(0.0);
        let right_px = (right * scale_x).max(0.0).max(left_px);
        let top_px = ((page_height - top) * scale_y).max(0.0);
        let bottom_px = ((page_height - bottom) * scale_y).max(0.0).max(top_px);

        let px = left_px.floor();
        let py = top_px.floor();
        let pw = right_px.ceil() - px;
        let ph = bottom_px.ceil() - py;
        Self::new(px as u32, py as u32, pw as u32, ph as u32)
    }
}

/// Where a glyph's geometry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "lowercase")
)]
#[non_exhaustive]
pub enum GlyphSource {
    /// From the PDF's own text layer (a text-drawing operator).
    Text,
    /// From OCR over the rendered pixels (a caller-supplied source).
    Ocr,
}

/// One glyph: the span of page text it covers and its box in rendered pixels.
///
/// `start`/`end` are UTF-16 code-unit offsets into the page's text, so a
/// detected span (also in UTF-16) selects the glyphs to redact.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Glyph {
    /// Start UTF-16 code-unit offset into the page text (inclusive).
    pub start: u32,
    /// End UTF-16 code-unit offset into the page text (exclusive).
    pub end: u32,
    /// The glyph's box in rendered-page pixels.
    pub rect: PixelRect,
    /// Where the geometry came from.
    pub source: GlyphSource,
}

/// A rendered page: its pixel dimensions, its text, and every glyph's box.
///
/// This is the observation the raster redaction consumes: `pixels` is the RGB8
/// image to overwrite, `text` is what detection ran over, and `glyphs` maps
/// detected UTF-16 spans back to pixel rectangles.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PageObservation {
    /// 1-based page number.
    pub page: u32,
    /// Rendered width in pixels.
    pub width: u32,
    /// Rendered height in pixels.
    pub height: u32,
    /// The page's text (the string `start`/`end` offsets index, in UTF-16).
    pub text: String,
    /// Every glyph's span and pixel box, in reading order.
    pub glyphs: Vec<Glyph>,
    /// Raw RGB8 pixels, `width * height * 3` bytes, row-major top-to-bottom.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub pixels: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::PixelRect;

    #[test]
    fn from_points_scales_and_flips_y() {
        // A 100pt-tall page rendered at 2x. A glyph box near the page top
        // (top=90pt) must land near the image top (small y).
        let r = PixelRect::from_points(10.0, 80.0, 30.0, 90.0, 100.0, 2.0, 2.0);
        assert_eq!(r.x, 20); // 10 * 2
        assert_eq!(r.width, 40); // (30-10) * 2
        assert_eq!(r.height, 20); // (90-80) * 2
        assert_eq!(r.y, 20); // (100 - 90) * 2, near the top
    }

    #[test]
    fn from_points_rounds_outward() {
        // Fractional edges: origin floors, size ceils, so the box never shrinks
        // inward and leaves original pixels uncovered.
        let r = PixelRect::from_points(10.4, 0.0, 20.6, 5.5, 100.0, 1.0, 1.0);
        assert_eq!(r.x, 10); // floor(10.4)
        assert!(r.x as f32 + r.width as f32 >= 20.6, "right edge covered");
    }

    #[test]
    fn from_points_covers_both_straddled_pixels() {
        // A box from x 10.5 to 11.4 touches pixel 10 and pixel 11. Deriving the
        // width from the ceiled far edge minus the floored origin covers both,
        // where a single ceiled width (ceil(0.9) = 1) would cover only pixel 10.
        let r = PixelRect::from_points(10.5, 0.0, 11.4, 1.0, 100.0, 1.0, 1.0);
        assert_eq!(r.x, 10, "origin floors to pixel 10");
        assert_eq!(r.width, 2, "covers pixels 10 and 11");
        assert!(r.x + r.width >= 12, "right edge past pixel 11");
    }

    #[test]
    fn from_points_clamps_negative_to_zero() {
        // A box off the left/top edge clamps to the image bounds, not negative.
        let r = PixelRect::from_points(-5.0, 0.0, 5.0, 200.0, 100.0, 1.0, 1.0);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0); // top=200 > page_height=100 -> clamped
    }
}
