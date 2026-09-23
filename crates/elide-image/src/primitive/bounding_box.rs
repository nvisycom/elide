//! [`BoundingBox`]: an axis-aligned rectangle over a coordinate scalar.

use elide_core::modality::Overlap;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Coordinate, Dimensions, Point, Polygon};

/// Axis-aligned rectangle, given by its minimum and maximum corners.
///
/// Generic over the coordinate scalar (a [`Coordinate`]): `BoundingBox<f64>` is a
/// fractional geometric *claim* (a recognizer's or model's possibly-out-of-bounds
/// region), while `BoundingBox<u32>` is a concrete set of image pixels, the corners
/// are exact indices, `min` inclusive and `max` exclusive, as an image crop reads
/// or paints. The two share the corner representation and the pure-comparison
/// operations ([`overlaps`], [`contains`]); the rest is scalar-specific, the float
/// box carries geometry (IoU, polygon, pixel clamping) and the pixel box the
/// integer indexing helpers.
///
/// [`min`] is the top-left corner and [`max`] the bottom-right under the usual
/// screen convention (y grows downward), though the box is agnostic to coordinate
/// orientation.
///
/// [`min`]: Self::min
/// [`max`]: Self::max
/// [`overlaps`]: Self::overlaps
/// [`contains`]: Self::contains
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BoundingBox<C: Coordinate> {
    /// Minimum corner (top-left, conventionally).
    pub min: Point<C>,
    /// Maximum corner (bottom-right, conventionally).
    pub max: Point<C>,
}

impl<C: Coordinate> BoundingBox<C> {
    /// Box spanning the two corners.
    pub const fn new(min: Point<C>, max: Point<C>) -> Self {
        Self { min, max }
    }

    /// Box from a top-left `origin` and a `size`: the corners are `origin`
    /// (inclusive) and `origin + size` (exclusive).
    ///
    /// For an integer (pixel) box the far corner saturates at the coordinate's
    /// maximum, so a far-off origin plus a large size does not overflow; such a
    /// box has no overlap with any real image and clamps to nothing.
    #[must_use]
    pub fn from_origin(origin: Point<C>, size: Dimensions<C>) -> Self {
        Self {
            min: origin,
            max: Point::new(origin.x.advance(size.width), origin.y.advance(size.height)),
        }
    }

    /// Whether this box overlaps `other`: they share interior area. Touching
    /// edges alone do not count.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.min.x < other.max.x
            && other.min.x < self.max.x
            && self.min.y < other.max.y
            && other.min.y < self.max.y
    }

    /// Whether this box fully contains `other`.
    pub fn contains(&self, other: &Self) -> bool {
        self.min.x <= other.min.x
            && self.min.y <= other.min.y
            && other.max.x <= self.max.x
            && other.max.y <= self.max.y
    }
}

impl BoundingBox<f64> {
    /// Clamp this box to the integer-pixel box lying inside an image of `dims`.
    ///
    /// Rounds *outward* — floors the minimum corner, ceils the maximum — then
    /// intersects with `[0, width) x [0, height)` and returns the resulting pixel
    /// box. Outward rounding is deliberate: this is what a redaction paints, so
    /// any pixel the float box touches at all must be covered; truncating instead
    /// could leave a fractional edge pixel visible. Returns `None` when the
    /// intersection is empty (the box lies past an edge, or spans no whole pixel),
    /// so a caller can `let region = bbox.to_pixels(dims)?;` and skip it.
    #[must_use]
    pub fn to_pixels(&self, dims: Dimensions<u32>) -> Option<BoundingBox<u32>> {
        let left = self.min.x.floor().clamp(0.0, f64::from(dims.width)) as u32;
        let top = self.min.y.floor().clamp(0.0, f64::from(dims.height)) as u32;
        let right = self.max.x.ceil().clamp(0.0, f64::from(dims.width)) as u32;
        let bottom = self.max.y.ceil().clamp(0.0, f64::from(dims.height)) as u32;
        if right <= left || bottom <= top {
            return None;
        }
        Some(BoundingBox::new(
            Point::new(left, top),
            Point::new(right, bottom),
        ))
    }

    /// Box width (`max.x - min.x`).
    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    /// Box height (`max.y - min.y`).
    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    /// Box area (`width * height`).
    pub fn area(&self) -> f64 {
        self.width() * self.height()
    }

    /// How this box sits against `other`, disjoint, one containing the other, or
    /// crossing with an area-IoU measure.
    pub fn overlap(&self, other: &Self) -> Overlap {
        if !self.overlaps(other) {
            return Overlap::Disjoint;
        }
        if self.contains(other) {
            return Overlap::Contains;
        }
        if other.contains(self) {
            return Overlap::ContainedBy;
        }
        let ix = (self.max.x.min(other.max.x) - self.min.x.max(other.min.x)).max(0.0);
        let iy = (self.max.y.min(other.max.y) - self.min.y.max(other.min.y)).max(0.0);
        let inter = ix * iy;
        let union = self.area() + other.area() - inter;
        Overlap::Crossing {
            iou: if union <= 0.0 {
                0.0
            } else {
                (inter / union) as f32
            },
        }
    }

    /// Smallest axis-aligned box covering both `self` and `other`.
    pub fn union(&self, other: &Self) -> Self {
        Self::new(
            Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        )
    }

    /// Box as a four-vertex [`Polygon`] (clockwise from the top-left corner under
    /// the usual screen convention).
    ///
    /// Lets a box be compared against a rotated or quadrilateral region through
    /// [`Polygon::overlaps`].
    pub fn to_polygon(&self) -> Polygon<f64> {
        Polygon::new(vec![
            self.min,
            Point::new(self.max.x, self.min.y),
            self.max,
            Point::new(self.min.x, self.max.y),
        ])
    }
}

impl BoundingBox<u32> {
    /// Width in pixels (`max.x - min.x`).
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.max.x.saturating_sub(self.min.x)
    }

    /// Height in pixels (`max.y - min.y`).
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.max.y.saturating_sub(self.min.y)
    }

    /// Left edge (inclusive): `min.x`.
    #[must_use]
    pub const fn left(&self) -> u32 {
        self.min.x
    }

    /// Top edge (inclusive): `min.y`.
    #[must_use]
    pub const fn top(&self) -> u32 {
        self.min.y
    }

    /// Right edge (exclusive): `max.x`.
    #[must_use]
    pub const fn right(&self) -> u32 {
        self.max.x
    }

    /// Bottom edge (exclusive): `max.y`.
    #[must_use]
    pub const fn bottom(&self) -> u32 {
        self.max.y
    }

    /// Pixel count covered by the box (`width * height`).
    #[must_use]
    pub const fn area(&self) -> u64 {
        self.width() as u64 * self.height() as u64
    }

    /// Whether the box has zero area.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    /// Size of the box as [`Dimensions`].
    #[must_use]
    pub const fn dimensions(&self) -> Dimensions<u32> {
        Dimensions::new(self.width(), self.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlaps_and_area() {
        let a = BoundingBox::from_origin(Point::new(0.0, 0.0), Dimensions::new(10.0, 10.0));
        let b = BoundingBox::from_origin(Point::new(5.0, 5.0), Dimensions::new(10.0, 10.0));
        assert!(a.overlaps(&b));
        assert_eq!(a.area(), 100.0);
        // Touching edge only: not an overlap.
        let c = BoundingBox::from_origin(Point::new(10.0, 0.0), Dimensions::new(5.0, 5.0));
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn to_pixels_clamps_inside_the_image() {
        let dims = Dimensions::new(100, 80);
        // A box partly past the right/bottom edge clamps to what fits.
        let b = BoundingBox::from_origin(Point::new(90.0, 70.0), Dimensions::new(50.0, 50.0));
        let region = b.to_pixels(dims).expect("partly inside");
        assert_eq!((region.left(), region.top()), (90, 70));
        assert_eq!((region.width(), region.height()), (10, 10));
        assert_eq!(region.right(), 100);
        assert_eq!(region.bottom(), 80);
    }

    #[test]
    fn to_pixels_rounds_outward() {
        let dims = Dimensions::new(100, 80);
        // A box `0.9..2.1` must cover every pixel it touches (0, 1, 2), so it
        // floors the min and ceils the max: `0..3`, not the truncated `0..1`.
        let frac = BoundingBox::new(Point::new(0.9, 0.9), Point::new(2.1, 2.1));
        let region = frac.to_pixels(dims).expect("inside");
        assert_eq!(
            (region.left(), region.top(), region.right(), region.bottom()),
            (0, 0, 3, 3)
        );
    }

    #[test]
    fn to_pixels_rejects_fully_outside_or_empty() {
        let dims = Dimensions::new(100, 80);
        // Origin past the edge: nothing inside.
        let outside = BoundingBox::from_origin(Point::new(100.0, 0.0), Dimensions::new(10.0, 10.0));
        assert_eq!(outside.to_pixels(dims), None);
        // Zero-size box clamps to empty.
        let empty = BoundingBox::from_origin(Point::new(10.0, 10.0), Dimensions::new(0.0, 0.0));
        assert_eq!(empty.to_pixels(dims), None);
        // Negative origin clamps to 0 and yields only the in-image part: the box
        // spans `-5..5`, so after clamping the left edge to 0 the region is
        // `0..5`, not `0..10`.
        let neg = BoundingBox::from_origin(Point::new(-5.0, -5.0), Dimensions::new(10.0, 10.0));
        let region = neg.to_pixels(dims).expect("partly inside");
        assert_eq!(
            (region.left(), region.top(), region.width(), region.height()),
            (0, 0, 5, 5)
        );
    }

    #[test]
    fn pixel_box_indexing() {
        let r = BoundingBox::from_origin(Point::new(90, 70), Dimensions::new(10, 10));
        assert_eq!(
            (r.left(), r.top(), r.right(), r.bottom()),
            (90, 70, 100, 80)
        );
        assert_eq!(r.area(), 100);
        assert!(!r.is_empty());
        assert_eq!(r.dimensions(), Dimensions::new(10, 10));
    }
}
