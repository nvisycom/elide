//! [`Layout`]: an image's recognized text laid out in space.
//!
//! What makes an image *recognizable*: a recognizer reads its
//! [`text`](Layout::text) like any other string, finds a match at a byte
//! range, and [`resolve`](Layout::resolve)s that range back to the
//! [`ImageLocation`] of the words it covers — via the per-word bounding
//! boxes the layout carries. Populated by an OCR pass today; the structure
//! can grow to carry richer layout (headings, tables, reading order). The
//! image counterpart of the audio [`Transcription`].
//!
//! [`Transcription`]: crate::modality::audio::Transcription

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::ImageLocation;
use crate::primitive::{BoundingBox, Confidence, Point};

/// Separator inserted between blocks when building the flat layout text, so
/// adjacent blocks don't run their words together.
const BLOCK_SEPARATOR: &str = "\n";

/// An image's recognized text, laid out in space.
///
/// An ordered set of [`LayoutBlock`]s (the recognized text regions). The flat
/// [`text`](Self::text) — the blocks joined — is what a recognizer
/// inspects; [`resolve`](Self::resolve) maps a byte range of that text back
/// to the [`ImageLocation`] it occupies, using the blocks' (and their
/// words') bounding boxes. Empty when the backend recognized nothing.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Layout {
    /// Blocks in reading order.
    blocks: Vec<LayoutBlock>,
    /// The blocks' text joined by [`BLOCK_SEPARATOR`], cached so recognition
    /// and byte-range resolution share one flat string.
    text: String,
}

/// One recognized region of an image: its bounding box and text, optionally
/// broken into per-word boxes.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LayoutBlock {
    /// Bounding region of the block in image coordinates.
    pub region: ImageLocation,
    /// Recognized text for this block.
    pub text: String,
    /// Per-word boxes within the block, when the backend emitted them.
    /// Empty otherwise; resolution then falls back to the block region.
    #[cfg_attr(feature = "serde", serde(default))]
    pub words: Vec<LayoutWord>,
}

/// One word within a [`LayoutBlock`], with its own bounding box.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LayoutWord {
    /// Bounding region of the word in image coordinates.
    pub region: ImageLocation,
    /// The word text, as it appears in the block text.
    pub text: String,
    /// Per-word confidence, when reported.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confidence: Option<Confidence>,
}

impl LayoutBlock {
    /// A block covering `region` with the given text and no per-word boxes.
    pub fn new(region: ImageLocation, text: impl Into<String>) -> Self {
        Self {
            region,
            text: text.into(),
            words: Vec::new(),
        }
    }

    /// Attach per-word boxes.
    #[must_use]
    pub fn with_words(mut self, words: Vec<LayoutWord>) -> Self {
        self.words = words;
        self
    }
}

impl LayoutWord {
    /// A word covering `region` with the given text and no confidence set.
    pub fn new(region: ImageLocation, text: impl Into<String>) -> Self {
        Self {
            region,
            text: text.into(),
            confidence: None,
        }
    }

    /// Attach a per-word confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

impl Layout {
    /// Build a layout from blocks, computing the flat text.
    #[must_use]
    pub fn new(blocks: Vec<LayoutBlock>) -> Self {
        let text = blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(BLOCK_SEPARATOR);
        Self { blocks, text }
    }

    /// The flat layout text a recognizer inspects: the blocks' text joined.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Blocks in reading order.
    #[must_use]
    pub fn blocks(&self) -> &[LayoutBlock] {
        &self.blocks
    }

    /// Whether the layout has no blocks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Byte offset where each block's text begins within [`text`](Self::text).
    fn block_offsets(&self) -> impl Iterator<Item = (usize, &LayoutBlock)> {
        let mut offset = 0;
        self.blocks.iter().map(move |block| {
            let start = offset;
            offset += block.text.len() + BLOCK_SEPARATOR.len();
            (start, block)
        })
    }

    /// Resolve a byte `range` of [`text`](Self::text) to the
    /// [`ImageLocation`] of the words it covers.
    ///
    /// Returns the union of the covered words' bounding boxes (the whole
    /// block region when a block has no per-word boxes), keeping the shared
    /// page. When a single word covers the range and carries a polygon, that
    /// polygon is preserved. `None` when the range covers no block (out of
    /// bounds, or an empty OCR).
    #[must_use]
    pub fn resolve(&self, range: Range<usize>) -> Option<ImageLocation> {
        let mut acc = RegionUnion::default();

        for (block_start, block) in self.block_offsets() {
            let block_end = block_start + block.text.len();
            // Skip blocks the range does not touch (half-open overlap).
            if range.start >= block_end || range.end <= block_start {
                continue;
            }

            let local_start = range.start.saturating_sub(block_start);
            let local_end = range.end.min(block_end).saturating_sub(block_start);

            if block.words.is_empty() {
                acc.add(&block.region);
            } else {
                acc.add_words(block, local_start..local_end);
            }
        }

        acc.into_location()
    }
}

/// Accumulates the bounding boxes of covered regions into one location.
#[derive(Default)]
struct RegionUnion {
    bbox: Option<BoundingBox>,
    page: Option<u32>,
    /// The single region added so far, kept so a lone covered word can pass
    /// its polygon through. Cleared once more than one region is unioned.
    sole: Option<ImageLocation>,
    count: usize,
}

impl RegionUnion {
    fn add(&mut self, location: &ImageLocation) {
        self.bbox = Some(match self.bbox.take() {
            Some(acc) => union(acc, location.bounding_box),
            None => location.bounding_box,
        });
        // First region sets the page; a later region on a different page is
        // a degenerate cross-page match — we keep the first page.
        if self.page.is_none() {
            self.page = location.page;
        }
        self.sole = if self.count == 0 {
            Some(location.clone())
        } else {
            None
        };
        self.count += 1;
    }

    /// Add the words of `block` whose byte extent overlaps the block-local
    /// `range`, walking each word's position in the block text.
    fn add_words(&mut self, block: &LayoutBlock, range: Range<usize>) {
        let mut search_from = 0;
        let mut matched = false;
        for word in &block.words {
            let Some(rel) = block.text[search_from..].find(word.text.as_str()) else {
                continue;
            };
            let word_start = search_from + rel;
            let word_end = word_start + word.text.len();
            search_from = word_end;

            if range.start >= word_end || range.end <= word_start {
                continue;
            }
            self.add(&word.region);
            matched = true;
        }
        // No word overlapped (e.g. a match inside inter-word whitespace):
        // fall back to the block region so the match still has an extent.
        if !matched {
            self.add(&block.region);
        }
    }

    fn into_location(self) -> Option<ImageLocation> {
        let bbox = self.bbox?;
        // A single covered region keeps its polygon; a union drops it (the
        // enclosing box is axis-aligned).
        let polygon = if self.count == 1 {
            self.sole.and_then(|l| l.polygon)
        } else {
            None
        };
        Some(ImageLocation {
            bounding_box: bbox,
            polygon,
            page: self.page,
        })
    }
}

/// The smallest axis-aligned box enclosing both inputs.
fn union(a: BoundingBox, b: BoundingBox) -> BoundingBox {
    BoundingBox::new(
        Point::new(a.min.x.min(b.min.x), a.min.y.min(b.min.y)),
        Point::new(a.max.x.max(b.max.x), a.max.y.max(b.max.y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    fn word(x: f64, y: f64, w: f64, h: f64, text: &str) -> LayoutWord {
        LayoutWord::new(loc(x, y, w, h), text)
    }

    /// "Call Alice" as one block of two boxed words.
    fn two_word_block() -> LayoutBlock {
        LayoutBlock::new(loc(0.0, 0.0, 100.0, 20.0), "Call Alice").with_words(vec![
            word(0.0, 0.0, 40.0, 20.0, "Call"),
            word(50.0, 0.0, 50.0, 20.0, "Alice"),
        ])
    }

    #[test]
    fn text_is_blocks_joined() {
        let t = Layout::new(vec![
            LayoutBlock::new(loc(0.0, 0.0, 10.0, 10.0), "hello"),
            LayoutBlock::new(loc(0.0, 20.0, 10.0, 10.0), "world"),
        ]);
        assert_eq!(t.text(), "hello\nworld");
    }

    #[test]
    fn resolve_maps_a_word_range_to_its_box() {
        let t = Layout::new(vec![two_word_block()]);
        // "Alice" is at bytes 5..10.
        let region = t.resolve(5..10).expect("in bounds");
        let bb = region.bounding_box;
        assert_eq!((bb.min.x, bb.min.y), (50.0, 0.0));
        assert_eq!((bb.max.x, bb.max.y), (100.0, 20.0));
    }

    #[test]
    fn resolve_unions_multiple_words() {
        let t = Layout::new(vec![two_word_block()]);
        // "Call Alice" -> bytes 0..10 -> union of both word boxes.
        let region = t.resolve(0..10).expect("in bounds");
        let bb = region.bounding_box;
        assert_eq!((bb.min.x, bb.min.y), (0.0, 0.0));
        assert_eq!((bb.max.x, bb.max.y), (100.0, 20.0));
    }

    #[test]
    fn resolve_falls_back_to_block_region_without_words() {
        let t = Layout::new(vec![LayoutBlock::new(
            loc(5.0, 5.0, 30.0, 10.0),
            "no word boxes",
        )]);
        let region = t.resolve(3..7).expect("in bounds");
        assert_eq!(region.bounding_box.min.x, 5.0);
    }

    #[test]
    fn resolve_out_of_bounds_is_none() {
        let t = Layout::new(vec![two_word_block()]);
        assert!(t.resolve(100..200).is_none());
    }

    #[test]
    fn resolve_on_empty_is_none() {
        let t = Layout::default();
        assert!(t.resolve(0..5).is_none());
    }
}
