//! Glyph geometry for raster redaction: a page's glyphs as PDFium reads them,
//! and the pixels a glyph covers once the page is rendered.

use crate::text::{GlyphRect, PageFrame, PageGeometry};

/// A page's text as PDFium reads it, with every glyph's box: the text
/// detection runs over, and the boxes a detected span selects.
#[derive(Debug, Clone)]
pub(crate) struct PageObservation {
    /// 1-based page number.
    pub(crate) page: u32,
    /// The page's text.
    pub(crate) text: String,
    /// Every glyph's character span and box in default user space, in reading
    /// order.
    pub(crate) glyphs: Vec<GlyphRect>,
    /// The page's visible box and rotation.
    pub(crate) frame: PageFrame,
}

impl PageObservation {
    /// The page's glyph boxes and frame.
    pub(crate) fn geometry(&self) -> PageGeometry<'_> {
        PageGeometry {
            rects: &self.glyphs,
            frame: &self.frame,
        }
    }
}

/// An axis-aligned rectangle in rendered-page pixels (top-left origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelRect {
    /// Left edge, in pixels from the page's left.
    pub(crate) x: u32,
    /// Top edge, in pixels from the page's top.
    pub(crate) y: u32,
    /// Width in pixels.
    pub(crate) width: u32,
    /// Height in pixels.
    pub(crate) height: u32,
}

impl PixelRect {
    /// The rectangle covering every pixel `corners` touch, with a pixel to
    /// spare on each side: each corner is the pixel a point was rounded to,
    /// which may sit inside the glyph's true edge, so nothing of the glyph is
    /// left uncovered.
    pub(crate) fn covering(corners: [(i32, i32); 4]) -> Self {
        let xs = corners.map(|(x, _)| x);
        let ys = corners.map(|(_, y)| y);
        let clamp = |v: i32| u32::try_from(v).unwrap_or(0);
        let (x0, x1) = (
            clamp(min(xs).saturating_sub(1)),
            clamp(max(xs)).saturating_add(2),
        );
        let (y0, y1) = (
            clamp(min(ys).saturating_sub(1)),
            clamp(max(ys)).saturating_add(2),
        );
        Self {
            x: x0,
            y: y0,
            width: x1.saturating_sub(x0),
            height: y1.saturating_sub(y0),
        }
    }
}

fn min(values: [i32; 4]) -> i32 {
    values.into_iter().min().unwrap_or(0)
}

fn max(values: [i32; 4]) -> i32 {
    values.into_iter().max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::PixelRect;

    #[test]
    fn covers_every_pixel_its_corners_touch() {
        // A rotated page maps corners out of order; the rect spans them all,
        // a pixel past each.
        let r = PixelRect::covering([(12, 40), (30, 40), (30, 20), (12, 20)]);
        assert_eq!(
            r,
            PixelRect {
                x: 11,
                y: 19,
                width: 21,
                height: 23
            }
        );
    }

    #[test]
    fn a_corner_off_the_page_clamps_to_its_edge() {
        let r = PixelRect::covering([(-5, -3), (4, -3), (4, 6), (-5, 6)]);
        assert_eq!(
            r,
            PixelRect {
                x: 0,
                y: 0,
                width: 6,
                height: 8
            }
        );
    }

    #[test]
    fn an_extreme_corner_does_not_overflow() {
        let r = PixelRect::covering([(i32::MIN, i32::MIN), (4, i32::MIN), (4, 6), (i32::MIN, 6)]);
        assert_eq!((r.x, r.y), (0, 0));
        assert_eq!((r.width, r.height), (6, 8));
    }
}
