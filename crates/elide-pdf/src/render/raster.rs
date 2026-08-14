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

/// A detected span to redact on a page: a UTF-16 range into the page's text.
///
/// The range selects the [`glyphs`](PageObservation::glyphs) whose boxes are
/// filled. Spans are matched against the same UTF-16 offsets the glyphs carry.
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
    /// A detection of `[start, end)` on `page`.
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

        // Validate every page buffer up front: an RGB8 buffer must be exactly
        // width * height * 3 bytes, or the fill would read/write out of bounds.
        for page in &pages {
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

        // Fail-closed: every detection must name a page present in `pages`.
        for d in detections {
            if !pages.iter().any(|p| p.page == d.page) {
                return Err(Error::unsafe_rewrite(format!(
                    "detection names page {} not in the observed pages",
                    d.page
                )));
            }
        }

        // Fill each detection's glyph boxes on its page.
        for page in &mut pages {
            for d in detections.iter().filter(|d| d.page == page.page) {
                for rect in glyph_rects(&page.glyphs, d.start, d.end) {
                    fill_rect(&mut page.pixels, page.width, page.height, rect, fill_rgb);
                }
            }
        }

        emit::emit(&self.source_bytes(), pages)
    }
}

/// The pixel boxes of every glyph whose span overlaps `[start, end)`.
fn glyph_rects(glyphs: &[Glyph], start: u32, end: u32) -> impl Iterator<Item = PixelRect> + '_ {
    glyphs
        .iter()
        .filter(move |g| g.start < end && g.end > start)
        .map(|g| g.rect)
}

/// Destructively overwrite `rect` in an RGB8 `pixels` buffer with `fill`,
/// clipped to the page bounds.
fn fill_rect(pixels: &mut [u8], width: u32, height: u32, rect: PixelRect, fill: [u8; 3]) {
    let x0 = rect.x.min(width);
    let y0 = rect.y.min(height);
    let x1 = rect.x.saturating_add(rect.width).min(width);
    let y1 = rect.y.saturating_add(rect.height).min(height);
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
