//! [`TextBlock`]: one page's decoded text with the map back to the glyph bytes
//! that drew it, the walker's output the redactor consumes.

use std::ops::Range;

use elide_core::primitive::UnitBoundingBox;
use lopdf::ObjectId;

use super::OffsetMap;
use super::geometry::PageFrame;
use super::walker::GlyphRect;

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

/// Whether two device boxes sit on the same text line: their vertical spans
/// overlap by more than half the smaller box's height.
fn same_line(a: &super::geometry::DeviceRect, b: &super::geometry::DeviceRect) -> bool {
    let overlap = a.max_y.min(b.max_y) - a.min_y.max(b.min_y);
    let min_h = (a.max_y - a.min_y)
        .min(b.max_y - b.min_y)
        .max(f64::MIN_POSITIVE);
    overlap > min_h * 0.5
}

/// One page's rendered glyph geometry, carried from extraction to the codec so a
/// detected entity's char span can be resolved to page-normalized rectangles.
/// Separate from the public [`Block`](crate::extract::Block) so geometry stays a
/// crate-internal concern.
#[derive(Debug, Clone)]
pub(crate) struct PageGeometry {
    /// 1-based page number.
    pub(crate) page: u32,
    /// Per-glyph device boxes, in draw order.
    pub(crate) rects: Vec<GlyphRect>,
    /// The page's visible rectangle and rotation, for normalization.
    pub(crate) frame: PageFrame,
}

impl PageGeometry {
    /// The page-normalized rectangles covering `chars`, one per line (see
    /// [`TextBlock::rects_for`]).
    pub(crate) fn rects_for(&self, chars: Range<usize>) -> Vec<UnitBoundingBox> {
        let block = TextBlockRef {
            rects: &self.rects,
            frame: &self.frame,
        };
        block.rects_for(chars)
    }
}

/// A borrowed view over a page's geometry, so the merge/normalize logic is
/// written once and shared by [`TextBlock`] and [`PageGeometry`].
struct TextBlockRef<'a> {
    rects: &'a [GlyphRect],
    frame: &'a PageFrame,
}

impl TextBlockRef<'_> {
    fn rects_for(&self, chars: Range<usize>) -> Vec<UnitBoundingBox> {
        let covering: Vec<_> = self
            .rects
            .iter()
            .filter(|g| g.chars.start < chars.end && g.chars.end > chars.start)
            .map(|g| g.rect)
            .collect();
        if covering.is_empty() {
            return Vec::new();
        }
        let mut lines: Vec<super::geometry::DeviceRect> = Vec::new();
        for rect in covering {
            match lines.last_mut() {
                Some(line) if same_line(line, &rect) => *line = line.union(rect),
                _ => lines.push(rect),
            }
        }
        lines.into_iter().map(|r| self.frame.normalize(r)).collect()
    }
}

/// A font referenced while walking content, resolved to its `/ToUnicode` CMap.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FontEntry {
    /// The font's `/ToUnicode` stream object id, or `None` when the font has no
    /// CMap (nothing to scrub).
    pub(crate) to_unicode: Option<ObjectId>,
}
