//! Raster redaction: destructively overwrite detected pixels and emit a fresh
//! image-only PDF.
//!
//! This is the redaction guarantee. A text-layer rewrite edits drawing
//! operators, which cannot reliably remove a value on subset/CID fonts and
//! leaves metadata, annotations, and prior revisions untouched. Raster
//! redaction instead works on the *rendered pixels*: it fills every detected
//! glyph's box with an opaque colour and rebuilds a new document whose only
//! content is the sanitised page images — **no object, stream, or byte from the
//! source is copied forward**, so nothing can survive underneath.
//!
//! The output is not searchable or accessible: it is images. That is the
//! deliberate trade for a guarantee that the original text is gone.

use super::{Glyph, PageObservation, PixelRect, emit};
use crate::error::{Error, Result};

/// A detected span to redact on a page: a **UTF-16 code-unit** range into the
/// page's text, matched against the rendered [`glyphs`](PageObservation::glyphs).
///
/// The range selects the glyphs whose boxes are filled; spans are matched
/// against the same UTF-16 offsets the glyphs carry.
///
/// This is **not** interchangeable with [`redact::Detection`](crate::redact::Detection):
/// that one's `start`/`end` are Unicode *character* offsets into a page's text
/// for the text-layer rewrite, whereas these are UTF-16 code-unit offsets into a
/// rendered [`PageObservation`]'s text. The two constructors look identical but
/// carry different offset units — pass the offsets that belong to this raster
/// path, not character offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    /// 1-based page number the span is on.
    pub page: u32,
    /// Start UTF-16 code-unit offset into the page text (inclusive).
    pub start: u32,
    /// End UTF-16 code-unit offset into the page text (exclusive).
    pub end: u32,
}

impl Detection {
    /// A detection of `[start, end)` on `page`, where `start`/`end` are UTF-16
    /// code-unit offsets into the rendered page's text.
    pub fn new(page: u32, start: u32, end: u32) -> Self {
        Self { page, start, end }
    }
}

impl super::Pdf {
    /// Redact `detections` by destructively overwriting their pixels in the
    /// rendered `pages`, then emit a fresh image-only PDF.
    ///
    /// `pages` are the observations from [`observe`](super::Pdf::observe) (or an
    /// equivalent with OCR-sourced glyphs for scanned pages). Each detection's
    /// glyph boxes are filled with `fill_rgb`; the output PDF's only content is
    /// the sanitised page images. No source object is copied forward.
    ///
    /// Returns the new document bytes and a [`Certificate`](super::Certificate) binding the source,
    /// the sanitised pixels, and the output by SHA-256.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a page's
    /// pixel buffer size does not match its dimensions, or a detection names a
    /// page not present in `pages`.
    #[cfg_attr(docsrs, doc(cfg(feature = "render")))]
    pub fn redact_raster(
        &self,
        pages: Vec<PageObservation>,
        detections: &[Detection],
        fill_rgb: [u8; 3],
    ) -> Result<(Vec<u8>, emit::Certificate)> {
        let mut pages = pages;

        validate_pages(&pages, detections)?;

        // Fill each detection's glyph boxes on its page.
        for page in &mut pages {
            for d in detections.iter().filter(|d| d.page == page.page) {
                for rect in glyph_rects(&page.glyphs, d.start, d.end) {
                    fill_rect(&mut page.pixels, page.width, page.height, rect, fill_rgb);
                }
            }
        }

        // The fill above set every covered pixel to `fill_rgb`; a mismatch here
        // would mean the fill and verify disagree on which pixels a detection
        // selects, which must never ship.
        #[cfg(feature = "test-utils")]
        debug_assert!(
            verify_raster_coverage(&pages, detections, fill_rgb).is_ok(),
            "redact_raster left a covered pixel unfilled"
        );

        emit::emit(&self.source_bytes(), pages)
    }
}

/// Verify that every detection's glyph boxes are painted `fill_rgb` in `pages`.
///
/// Returns `Ok(())` iff, for every detection, every pixel of every glyph box it
/// selects — by the same span-overlap rule and page-bounds clipping
/// [`redact_raster`](super::Pdf::redact_raster) fills with — is exactly
/// `fill_rgb` in that page's [`pixels`](PageObservation::pixels).
///
/// A raster redaction's output is an image-only PDF with no text layer, so
/// re-observing the output yields no glyphs and the rects cannot be re-derived
/// from it. Verification therefore runs against the observations *after* the
/// fill: they carry both the glyphs and the now-painted pixels. A caller (or an
/// end-to-end test) uses this to confirm a redaction actually painted the
/// pixels it was meant to, without re-implementing the coordinate math.
///
/// # Errors
///
/// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a page's
/// pixel buffer size does not match its dimensions, a detection names a page
/// not present in `pages`, or a covered pixel is not `fill_rgb` (naming the
/// page and the offending pixel).
#[cfg(feature = "test-utils")]
#[cfg_attr(docsrs, doc(cfg(feature = "test-utils")))]
pub fn verify_raster_coverage(
    pages: &[PageObservation],
    detections: &[Detection],
    fill_rgb: [u8; 3],
) -> Result<()> {
    validate_pages(pages, detections)?;

    for page in pages {
        for d in detections.iter().filter(|d| d.page == page.page) {
            for rect in glyph_rects(&page.glyphs, d.start, d.end) {
                if let Some((x, y)) = unfilled_pixel(&page.pixels, page.width, rect, fill_rgb) {
                    return Err(Error::unsafe_rewrite(format!(
                        "page {} pixel ({x}, {y}) in glyph box {rect:?} is not the fill colour {fill_rgb:?}",
                        page.page
                    )));
                }
            }
        }
    }

    Ok(())
}

/// Validate every page buffer and detection target, fail-closed.
///
/// An RGB8 buffer must be exactly `width * height * 3` bytes, or the fill would
/// read/write out of bounds; every detection must name a page present in
/// `pages`.
fn validate_pages(pages: &[PageObservation], detections: &[Detection]) -> Result<()> {
    for page in pages {
        let expected = (page.width as usize)
            .checked_mul(page.height as usize)
            .and_then(|n| n.checked_mul(3));
        if expected != Some(page.pixels.len()) {
            return Err(Error::unsafe_rewrite(format!(
                "page {} pixel buffer is {} bytes, expected {:?}",
                page.page,
                page.pixels.len(),
                expected
            )));
        }
    }

    for d in detections {
        if !pages.iter().any(|p| p.page == d.page) {
            return Err(Error::unsafe_rewrite(format!(
                "detection names page {} not in the observed pages",
                d.page
            )));
        }
    }

    Ok(())
}

/// The pixel boxes of every glyph whose span overlaps `[start, end)`.
fn glyph_rects(glyphs: &[Glyph], start: u32, end: u32) -> impl Iterator<Item = PixelRect> + '_ {
    glyphs
        .iter()
        .filter(move |g| g.start < end && g.end > start)
        .map(|g| g.rect)
}

/// Clip `rect` to a `width` x `height` page: the half-open pixel ranges
/// `x0..x1` by `y0..y1` it actually covers.
fn clip_rect(width: u32, height: u32, rect: PixelRect) -> (u32, u32, u32, u32) {
    let x0 = rect.x.min(width);
    let y0 = rect.y.min(height);
    let x1 = rect.x.saturating_add(rect.width).min(width);
    let y1 = rect.y.saturating_add(rect.height).min(height);
    (x0, y0, x1, y1)
}

/// Destructively overwrite `rect` in an RGB8 `pixels` buffer with `fill`,
/// clipped to the page bounds.
fn fill_rect(pixels: &mut [u8], width: u32, height: u32, rect: PixelRect, fill: [u8; 3]) {
    let (x0, y0, x1, y1) = clip_rect(width, height, rect);
    for y in y0..y1 {
        let row = (y as usize) * (width as usize) * 3;
        for x in x0..x1 {
            let i = row + (x as usize) * 3;
            pixels[i] = fill[0];
            pixels[i + 1] = fill[1];
            pixels[i + 2] = fill[2];
        }
    }
}

/// The first pixel of `rect` (clipped to `width`) not equal to `fill` in an
/// RGB8 `pixels` buffer, as `(x, y)`; `None` if every covered pixel matches.
///
/// The page height is derived from the buffer length, so the same page-bounds
/// clipping [`fill_rect`] applies holds — an out-of-bounds glyph box covers no
/// pixels rather than reporting a false mismatch.
#[cfg(feature = "test-utils")]
fn unfilled_pixel(pixels: &[u8], width: u32, rect: PixelRect, fill: [u8; 3]) -> Option<(u32, u32)> {
    let height = if width == 0 {
        0
    } else {
        (pixels.len() / 3 / width as usize) as u32
    };
    let (x0, y0, x1, y1) = clip_rect(width, height, rect);
    for y in y0..y1 {
        let row = (y as usize) * (width as usize) * 3;
        for x in x0..x1 {
            let i = row + (x as usize) * 3;
            if pixels[i..i + 3] != fill {
                return Some((x, y));
            }
        }
    }
    None
}

#[cfg(all(test, feature = "test-utils"))]
mod tests {
    use super::{Detection, verify_raster_coverage};
    use crate::error::ErrorKind;
    use crate::render::{Glyph, GlyphSource, PageObservation, PixelRect};

    const FILL: [u8; 3] = [255, 0, 0];

    /// A page filled entirely with `rgb`, carrying `glyphs`.
    fn page(
        page: u32,
        width: u32,
        height: u32,
        rgb: [u8; 3],
        glyphs: Vec<Glyph>,
    ) -> PageObservation {
        let pixels = rgb.repeat((width as usize) * (height as usize));
        PageObservation {
            page,
            width,
            height,
            text: String::new(),
            glyphs,
            pixels,
        }
    }

    /// A text-sourced glyph spanning `[start, end)` with box `rect`.
    fn glyph(start: u32, end: u32, rect: PixelRect) -> Glyph {
        Glyph {
            start,
            end,
            rect,
            source: GlyphSource::Text,
        }
    }

    #[test]
    fn ok_when_every_covered_pixel_is_fill() {
        let g = glyph(0, 1, PixelRect::new(1, 1, 2, 2));
        let mut p = page(1, 4, 4, [0, 0, 0], vec![g]);
        // Paint exactly the glyph box with the fill colour.
        for y in 1..3 {
            for x in 1..3 {
                let i = (y * 4 + x) * 3;
                p.pixels[i..i + 3].copy_from_slice(&FILL);
            }
        }

        let detections = [Detection::new(1, 0, 1)];
        assert!(verify_raster_coverage(&[p], &detections, FILL).is_ok());
    }

    #[test]
    fn err_when_a_covered_pixel_is_not_fill() {
        // A page left entirely unpainted: the covered box is still black.
        let g = glyph(0, 1, PixelRect::new(1, 1, 2, 2));
        let p = page(1, 4, 4, [0, 0, 0], vec![g]);

        let detections = [Detection::new(1, 0, 1)];
        let err = verify_raster_coverage(&[p], &detections, FILL).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
    }

    #[test]
    fn err_when_detection_names_absent_page() {
        let p = page(1, 2, 2, FILL, vec![]);

        let detections = [Detection::new(2, 0, 1)];
        let err = verify_raster_coverage(&[p], &detections, FILL).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
    }

    #[test]
    fn partial_clip_is_not_a_false_failure() {
        // A glyph box straddling the right/bottom edge: only pixels inside the
        // page are checked. The whole 2x2 page is fill, so the clipped box
        // matches even though the box extends past the bounds.
        let g = glyph(0, 1, PixelRect::new(1, 1, 4, 4));
        let p = page(1, 2, 2, FILL, vec![g]);

        let detections = [Detection::new(1, 0, 1)];
        assert!(verify_raster_coverage(&[p], &detections, FILL).is_ok());
    }
}
