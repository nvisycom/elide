//! Raster redaction: select the glyph boxes a detection covers, and paint them
//! over in a rendered page.
//!
//! The page is then written back as that one image
//! ([`reflatten`](super::reflatten())), so none of its text, vector content, or
//! images survives underneath.

use std::collections::BTreeMap;

use elide_core::{Error, Result};
use image::{Rgb, RgbImage};

use super::{GlyphBox, PageObservation, PixelRect};
use crate::text::Detection;

/// The colour a redacted glyph box is painted.
const FILL: Rgb<u8> = Rgb([0, 0, 0]);

/// The glyph boxes `detections` cover, keyed by page.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a detection
/// names a page not in `pages`, or selects no glyph: a span that would paint
/// nothing is a redaction that cannot be carried out, not a no-op.
pub(crate) fn glyph_boxes(
    pages: &[PageObservation],
    detections: &[Detection],
) -> Result<BTreeMap<u32, Vec<GlyphBox>>> {
    let mut boxes: BTreeMap<u32, Vec<GlyphBox>> = BTreeMap::new();
    for d in detections {
        let page = pages.iter().find(|p| p.page == d.page).ok_or_else(|| {
            Error::redaction(format!(
                "detection names page {}, which has no text",
                d.page
            ))
        })?;
        let selected: Vec<GlyphBox> = page
            .glyphs
            .iter()
            .filter(|g| g.start < d.end && g.end > d.start)
            .map(|g| g.bounds)
            .collect();
        if selected.is_empty() {
            return Err(Error::redaction(format!(
                "detection on page {} (chars {}..{}) selects no glyph to redact",
                d.page, d.start, d.end
            )));
        }
        boxes.entry(d.page).or_default().extend(selected);
    }
    Ok(boxes)
}

/// Paint `rect` over `image`, clipped to its bounds.
pub(crate) fn fill(image: &mut RgbImage, rect: PixelRect) {
    let x1 = rect.x.saturating_add(rect.width).min(image.width());
    let y1 = rect.y.saturating_add(rect.height).min(image.height());
    for y in rect.y..y1 {
        for x in rect.x..x1 {
            image.put_pixel(x, y, FILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use image::{Rgb, RgbImage};

    use super::*;
    use crate::render::Glyph;

    fn page(number: u32, glyphs: Vec<Glyph>) -> PageObservation {
        PageObservation {
            page: number,
            text: String::new(),
            glyphs,
        }
    }

    fn glyph(start: usize, end: usize, left: f32) -> Glyph {
        Glyph {
            start,
            end,
            bounds: GlyphBox {
                left,
                bottom: 0.0,
                right: left + 1.0,
                top: 1.0,
            },
        }
    }

    #[test]
    fn a_detection_selects_the_glyphs_it_overlaps() {
        let pages = [page(
            1,
            vec![glyph(0, 1, 0.0), glyph(1, 2, 1.0), glyph(2, 3, 2.0)],
        )];
        let boxes = glyph_boxes(&pages, &[Detection::new(1, 1, 3)]).unwrap();
        let lefts: Vec<f32> = boxes[&1].iter().map(|b| b.left).collect();
        assert_eq!(lefts, [1.0, 2.0]);
    }

    #[test]
    fn a_detection_on_a_page_without_text_is_an_error() {
        let pages = [page(1, vec![glyph(0, 1, 0.0)])];
        let err = glyph_boxes(&pages, &[Detection::new(2, 0, 1)]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    #[test]
    fn a_detection_that_selects_no_glyph_is_an_error() {
        let pages = [page(1, vec![glyph(5, 6, 0.0)])];
        let err = glyph_boxes(&pages, &[Detection::new(1, 0, 1)]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    #[test]
    fn a_fill_paints_its_rect_clipped_to_the_page() {
        let mut image = RgbImage::from_pixel(4, 4, Rgb([255, 255, 255]));
        fill(
            &mut image,
            PixelRect {
                x: 2,
                y: 1,
                width: 9,
                height: 2,
            },
        );
        for (x, y, pixel) in image.enumerate_pixels() {
            let inside = x >= 2 && (1..3).contains(&y);
            assert_eq!(*pixel == FILL, inside, "pixel ({x}, {y})");
        }
    }
}
