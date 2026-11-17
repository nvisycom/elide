//! [`TextBlock`]: one page's decoded text with the map back to the glyph bytes
//! that drew it, the walker's output the redactor consumes.

use std::ops::Range;

use elide_core::primitive::UnitBoundingBox;
use lopdf::ObjectId;

use super::OffsetMap;
use super::geometry::{DeviceRect, GlyphRect, PageFrame};

/// One page's recovered text, plus the correspondence back to the glyphs that
/// drew it.
///
/// `text` is the decoded page text a caller runs detection over (character
/// offsets); `offsets` maps a detected character range to the glyph bytes to
/// delete. `page_id` addresses the page object for content get/set, and `fonts`
/// carries the per-glyph font table the `/ToUnicode` scrub needs. Text drawn
/// through a `Do`-invoked Form XObject is included, so a detected span is located
/// and redactable wherever it was drawn.
#[derive(Debug, Clone)]
pub(crate) struct TextBlock {
    /// 1-based page number.
    pub(crate) page: u32,
    /// The page's indirect-object id, addressing the page object for content
    /// get/set (distinct from the 1-based `page` number).
    pub(crate) page_id: ObjectId,
    /// The decoded page text, in character order.
    pub(crate) text: String,
    /// The decoded-character-to-glyph-bytes correspondence for `text`.
    pub(crate) offsets: OffsetMap,
    /// Per-glyph device-space boxes (char range → box), in draw order, for
    /// resolving a detected char span to its rendered page rectangles.
    pub(crate) rects: Vec<GlyphRect>,
    /// The page's visible rectangle and rotation, for normalizing a device box
    /// to page fractions.
    pub(crate) frame: PageFrame,
    /// The fonts referenced by the glyphs in `offsets`, indexed by
    /// [`GlyphBytes::font`](super::GlyphBytes); each carries the font's
    /// `/ToUnicode` CMap object id (when it has one) so a deleted glyph's code can
    /// be scrubbed from the right CMap even when a font name recurs across
    /// resource scopes.
    pub(crate) fonts: Vec<FontEntry>,
}

impl TextBlock {
    /// The page's glyph boxes and frame.
    pub(crate) fn geometry(&self) -> PageGeometry<'_> {
        PageGeometry {
            rects: &self.rects,
            frame: &self.frame,
        }
    }
}

/// A page's glyph boxes in default user space and the frame that places them
/// on the page as displayed: what a detected span's rectangles and a reviewer's
/// mark are resolved against, whichever reader produced the glyphs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageGeometry<'a> {
    pub(crate) rects: &'a [GlyphRect],
    pub(crate) frame: &'a PageFrame,
}

impl<'a> PageGeometry<'a> {
    /// The page-normalized rectangles covering the characters `chars`, one
    /// per line: the boxes of the glyphs that drew them, merged along each
    /// line.
    pub(crate) fn rects_for(&self, chars: Range<usize>) -> Vec<UnitBoundingBox> {
        let mut lines: Vec<DeviceRect> = Vec::new();
        let covering = self
            .rects
            .iter()
            .filter(|g| g.chars.start < chars.end && g.chars.end > chars.start);
        for glyph in covering {
            match lines.last_mut() {
                Some(line) if same_line(line, &glyph.rect) => *line = line.union(glyph.rect),
                _ => lines.push(glyph.rect),
            }
        }
        lines.into_iter().map(|r| self.frame.normalize(r)).collect()
    }

    /// The page-local character range of each glyph whose center falls in any
    /// of `regions` (page-normalized, top-left origin), in draw order.
    pub(crate) fn glyphs_in(
        self,
        regions: &'a [UnitBoundingBox],
    ) -> impl Iterator<Item = Range<usize>> + 'a {
        let frame = self.frame;
        self.rects
            .iter()
            .filter(move |glyph| {
                let rect = frame.normalize(glyph.rect);
                let x = rect.x + rect.width / 2.0;
                let y = rect.y + rect.height / 2.0;
                regions.iter().any(|region| {
                    (region.x..=region.x + region.width).contains(&x)
                        && (region.y..=region.y + region.height).contains(&y)
                })
            })
            .map(|glyph| glyph.chars.clone())
    }
}

/// Whether two device boxes sit on the same text line: their vertical spans
/// overlap by more than half the smaller box's height.
fn same_line(a: &DeviceRect, b: &DeviceRect) -> bool {
    let overlap = a.max_y.min(b.max_y) - a.min_y.max(b.min_y);
    let min_h = (a.max_y - a.min_y)
        .min(b.max_y - b.min_y)
        .max(f64::MIN_POSITIVE);
    overlap > min_h * 0.5
}

/// A font referenced while walking content, resolved to its `/ToUnicode` CMap.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FontEntry {
    /// The font's `/ToUnicode` stream object id, or `None` when the font has no
    /// CMap (nothing to scrub).
    pub(crate) to_unicode: Option<ObjectId>,
}
