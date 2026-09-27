//! [`Layout`]: an image's recognized text laid out in space.
//!
//! What makes an image *recognizable*: a recognizer reads its [`text`] like any
//! other string, finds a match at a byte range, and [`resolve`]s that range back
//! to the [`ImageLocation`] of the regions it covers, via the per-region
//! bounding boxes the layout carries. Populated by an OCR pass today. The image
//! counterpart of the audio `Transcription`.
//!
//! A layout is a flat, ordered list of [`LayoutRegion`]s — each a run of
//! recognized text with its own box, the atom an OCR engine emits (a word, or a
//! whole line the backend did not split). The flat [`text`] is those regions
//! joined by a space, computed once, with each region's byte offset recorded so
//! [`resolve`] maps a range back to boxes without re-scanning the text.
//!
//! [`text`]: Layout::text
//! [`resolve`]: Layout::resolve

use std::ops::Range;

use elide_core::modality::ModalityArtifact;
use elide_core::primitive::Confidence;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::ImageLocation;

/// Separator inserted between regions when building the flat layout text, so
/// adjacent regions don't run their text together.
const REGION_SEPARATOR: &str = " ";

/// An image's recognized text, laid out in space.
///
/// A flat, ordered list of [`LayoutRegion`]s (the recognized text runs in
/// reading order). The flat [`text`], the regions joined by a space, is what a
/// recognizer inspects; [`resolve`] maps a byte range of that text back to the
/// [`ImageLocation`] it occupies, using the regions' boxes. Empty when the
/// backend recognized nothing.
///
/// [`text`]: Self::text
/// [`resolve`]: Self::resolve
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Layout {
    /// Regions in reading order.
    regions: Vec<LayoutRegion>,
    /// The regions' text joined by [`REGION_SEPARATOR`], cached so recognition
    /// and byte-range resolution share one flat string.
    text: String,
}

/// One recognized run of image text: its box, the text, and an optional
/// confidence.
///
/// The atom an OCR backend emits — typically a word, but a backend may report a
/// coarser run (a whole line) as one region; the layout treats them uniformly.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LayoutRegion {
    /// Bounding region of the text in image coordinates.
    pub region: ImageLocation,
    /// The recognized text of this region.
    pub text: String,
    /// Recognition confidence, when the backend reported one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confidence: Option<Confidence>,
}

impl LayoutRegion {
    /// A region covering `region` with the given text and no confidence set.
    pub fn new(region: ImageLocation, text: impl Into<String>) -> Self {
        Self {
            region,
            text: text.into(),
            confidence: None,
        }
    }

    /// Attach a recognition confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

impl Layout {
    /// Build a layout from regions, computing the flat text.
    #[must_use]
    pub fn new(regions: Vec<LayoutRegion>) -> Self {
        let text = regions
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join(REGION_SEPARATOR);
        Self { regions, text }
    }

    /// The flat layout text a recognizer inspects: the regions' text joined.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Regions in reading order.
    #[must_use]
    pub fn regions(&self) -> &[LayoutRegion] {
        &self.regions
    }

    /// Whether the layout has no regions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Byte offset where each region's text begins within [`text`].
    ///
    /// Mirrors how `text` is built: region `i` starts after the previous
    /// regions plus one separator each.
    ///
    /// [`text`]: Self::text
    fn region_offsets(&self) -> impl Iterator<Item = (usize, &LayoutRegion)> {
        let mut offset = 0;
        self.regions.iter().map(move |region| {
            let start = offset;
            offset += region.text.len() + REGION_SEPARATOR.len();
            (start, region)
        })
    }

    /// Resolve a byte `range` of [`text`] to the [`ImageLocation`] it covers:
    /// the union of the boxes of every region the range overlaps.
    ///
    /// A single covered region keeps its polygon; a union of several drops it,
    /// as the enclosing box is axis-aligned. `None` when the range overlaps no
    /// region (out of bounds, or an empty layout), so the caller drops the
    /// match rather than emit a placeless entity.
    ///
    /// [`text`]: Self::text
    #[must_use]
    pub fn resolve(&self, range: Range<usize>) -> Option<ImageLocation> {
        let mut union: Option<ImageLocation> = None;
        let mut count = 0usize;

        for (region_start, region) in self.region_offsets() {
            let region_end = region_start + region.text.len();
            // Skip regions the range does not touch (half-open overlap).
            if range.start >= region_end || range.end <= region_start {
                continue;
            }

            union = Some(match union {
                None => region.region.clone(),
                Some(acc) => {
                    let bbox = acc.bounding_box.union(&region.region.bounding_box);
                    // Keep the first region's page; a later region on a
                    // different page is a degenerate cross-page match.
                    let mut merged = ImageLocation::new(bbox);
                    if let Some(page) = acc.page {
                        merged = merged.with_page(page);
                    }
                    merged
                }
            });
            count += 1;
        }

        let mut location = union?;
        // A lone covered region passes its polygon through; a union cannot.
        if count > 1 {
            location.polygon = None;
        }
        Some(location)
    }
}

impl ModalityArtifact for Layout {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::{BoundingBox, Dimensions, Point};

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    fn region(x: f64, y: f64, w: f64, h: f64, text: &str) -> LayoutRegion {
        LayoutRegion::new(loc(x, y, w, h), text)
    }

    /// "Call Alice" as two boxed regions.
    fn two_region_layout() -> Layout {
        Layout::new(vec![
            region(0.0, 0.0, 40.0, 20.0, "Call"),
            region(50.0, 0.0, 50.0, 20.0, "Alice"),
        ])
    }

    #[test]
    fn text_is_regions_joined() {
        let l = Layout::new(vec![
            region(0.0, 0.0, 10.0, 10.0, "hello"),
            region(0.0, 20.0, 10.0, 10.0, "world"),
        ]);
        assert_eq!(l.text(), "hello world");
    }

    #[test]
    fn resolve_maps_a_region_range_to_its_box() {
        let l = two_region_layout();
        // "Alice" is at bytes 5..10.
        let region = l.resolve(5..10).expect("in bounds");
        let bb = region.bounding_box;
        assert_eq!((bb.min.x, bb.min.y), (50.0, 0.0));
        assert_eq!((bb.max.x, bb.max.y), (100.0, 20.0));
    }

    #[test]
    fn resolve_unions_multiple_regions() {
        let l = two_region_layout();
        // "Call Alice" -> bytes 0..10 -> union of both boxes.
        let region = l.resolve(0..10).expect("in bounds");
        let bb = region.bounding_box;
        assert_eq!((bb.min.x, bb.min.y), (0.0, 0.0));
        assert_eq!((bb.max.x, bb.max.y), (100.0, 20.0));
    }

    #[test]
    fn resolve_covers_every_region_the_range_touches() {
        // A match spanning two regions covers both boxes — no part is left
        // visible even when the regions have unequal widths.
        let l = Layout::new(vec![
            region(0.0, 0.0, 40.0, 20.0, "Alice"),
            region(50.0, 0.0, 60.0, 20.0, "Smith"),
        ]);
        // "Alice Smith" -> bytes 0..11.
        let region = l.resolve(0..11).expect("in bounds");
        assert_eq!(
            (region.bounding_box.min.x, region.bounding_box.max.x),
            (0.0, 110.0)
        );
    }

    #[test]
    fn resolve_out_of_bounds_is_none() {
        let l = two_region_layout();
        assert!(l.resolve(100..200).is_none());
    }

    #[test]
    fn resolve_on_empty_is_none() {
        let l = Layout::default();
        assert!(l.resolve(0..5).is_none());
    }
}
