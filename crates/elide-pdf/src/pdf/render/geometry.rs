//! A page's glyphs as PDFium reads them.

use crate::pdf::reader::{GlyphRect, PageFrame, PageGeometry};

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
