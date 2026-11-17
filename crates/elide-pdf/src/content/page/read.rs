//! Rendering a PDF's pages as the images that replace them: with the glyph
//! boxes detected text covers painted over, or a scanned page with the
//! regions redacted on it.

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, Result};
use image::{Rgb, RgbImage};

use super::{PageReplacement, ScannedPage};
use crate::content::text::Detection;
use crate::pdf::Pdf;
use crate::pdf::reader::DeviceRect;
use crate::pdf::render::{PageObservation, each_page};

/// The pages `pages` rendered at `scale` with the glyph boxes `boxes` names
/// for each painted over; see [`Flatten`](super::Flatten).
pub(super) fn flatten(
    pdf: &Pdf,
    pages: &BTreeSet<u32>,
    scale: f32,
    boxes: &BTreeMap<u32, Vec<DeviceRect>>,
) -> Result<Vec<PageReplacement>> {
    let mut out = Vec::new();
    each_page(pdf, pages, scale, |number, mut image, place| {
        for glyph in boxes.get(&number).into_iter().flatten() {
            let mut corners = [(0, 0); 4];
            for (corner, (x, y)) in corners.iter_mut().zip(glyph.corners()) {
                *corner = place(x, y)?;
            }
            fill(&mut image, PixelRect::covering(corners));
        }
        out.push(PageReplacement::pixels(number, image)?);
        Ok(())
    })?;
    Ok(out)
}

/// The `scanned` pages rendered fresh at `scale`, each with the regions
/// redacted on it painted over; see [`Repaint`](super::Repaint).
pub(super) fn repaint(
    pdf: &Pdf,
    scanned: &[ScannedPage],
    scale: f32,
) -> Result<Vec<PageReplacement>> {
    let numbers: BTreeSet<u32> = scanned.iter().map(|page| page.number).collect();
    let mut out = Vec::new();
    each_page(pdf, &numbers, scale, |number, image, _| {
        let page = scanned
            .iter()
            .find(|page| page.number == number)
            .ok_or_else(|| Error::redaction(format!("page {number} was not asked for")))?;
        out.push(page.repainted(image)?);
        Ok(())
    })?;
    Ok(out)
}

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
) -> Result<BTreeMap<u32, Vec<DeviceRect>>> {
    let mut boxes: BTreeMap<u32, Vec<DeviceRect>> = BTreeMap::new();
    for d in detections {
        let page = pages.iter().find(|p| p.page == d.page).ok_or_else(|| {
            Error::redaction(format!(
                "detection names page {}, which has no text",
                d.page
            ))
        })?;
        let selected: Vec<DeviceRect> = page
            .glyphs
            .iter()
            .filter(|g| g.chars.start < d.end && g.chars.end > d.start)
            .map(|g| g.rect)
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

/// The colour a redacted glyph box is painted.
const FILL: Rgb<u8> = Rgb([0, 0, 0]);

/// An axis-aligned rectangle in rendered-page pixels (top-left origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PixelRect {
    /// Left edge, in pixels from the page's left.
    pub(super) x: u32,
    /// Top edge, in pixels from the page's top.
    pub(super) y: u32,
    /// Width in pixels.
    pub(super) width: u32,
    /// Height in pixels.
    pub(super) height: u32,
}

impl PixelRect {
    /// The rectangle covering every pixel `corners` touch, with a pixel to
    /// spare on each side: each corner is the pixel a point was rounded to,
    /// which may sit inside the glyph's true edge, so nothing of the glyph is
    /// left uncovered.
    pub(super) fn covering(corners: [(i32, i32); 4]) -> Self {
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

/// Paint `rect` over `image`, clipped to its bounds.
pub(super) fn fill(image: &mut RgbImage, rect: PixelRect) {
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

    use super::*;
    use crate::pdf::reader::{GlyphRect, PageFrame};

    fn page(number: u32, glyphs: Vec<GlyphRect>) -> PageObservation {
        PageObservation {
            page: number,
            text: String::new(),
            glyphs,
            frame: PageFrame::LETTER,
        }
    }

    fn glyph(start: usize, end: usize, left: f64) -> GlyphRect {
        GlyphRect {
            chars: start..end,
            rect: DeviceRect {
                min_x: left,
                min_y: 0.0,
                max_x: left + 1.0,
                max_y: 1.0,
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
        let lefts: Vec<f64> = boxes[&1].iter().map(|b| b.min_x).collect();
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
