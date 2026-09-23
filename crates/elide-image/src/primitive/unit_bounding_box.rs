//! [`UnitBoundingBox`]: an axis-aligned box in normalized `0.0..=1.0`
//! coordinates.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{BoundingBox, Dimensions, Point};

/// Axis-aligned bounding box in normalized `0.0..=1.0` coordinates.
///
/// Used at API boundaries where the producer doesn't know the image's pixel
/// size, most commonly the output of vision-language models that never see the
/// original resolution. `(0, 0)` is the top-left corner of the image and
/// `(1, 1)` the bottom-right.
///
/// A newtype over a [`BoundingBox`] whose corners are read as image fractions
/// rather than pixels: it shares the box's representation but is a distinct
/// type, so a normalized box cannot be mistaken for a pixel-space one. Its only
/// operation is the [`denormalize`] / [`normalize`] bridge to pixel space (a
/// normalized box never participates in overlap or union, which happen in pixel
/// space). Field values are not clamped to `0.0..=1.0`; the type carries the
/// *intent* of unit-square coordinates, not a hard invariant, and conversion is
/// mechanical multiplication regardless.
///
/// [`denormalize`]: Self::denormalize
/// [`normalize`]: Self::normalize
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct UnitBoundingBox(BoundingBox<f64>);

impl UnitBoundingBox {
    /// Normalized box from a top-left origin `(x, y)` and a size, all in
    /// `0.0..=1.0` image fractions.
    #[must_use]
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(width, height),
        ))
    }

    /// Scale into a pixel-space [`BoundingBox`] for an image of `dims`,
    /// multiplying each axis by the matching dimension.
    #[must_use]
    pub fn denormalize(&self, dims: Dimensions<u32>) -> BoundingBox<f64> {
        let w = f64::from(dims.width);
        let h = f64::from(dims.height);
        BoundingBox::new(
            Point::new(self.0.min.x * w, self.0.min.y * h),
            Point::new(self.0.max.x * w, self.0.max.y * h),
        )
    }

    /// Build a unit box from a pixel-space [`BoundingBox`] on an image of
    /// `dims`, dividing each axis by the matching dimension.
    ///
    /// The inverse of [`denormalize`]. A zero `dims` axis yields a zero on that
    /// axis rather than a non-finite value.
    ///
    /// [`denormalize`]: Self::denormalize
    #[must_use]
    pub fn normalize(bbox: &BoundingBox<f64>, dims: Dimensions<u32>) -> Self {
        let w = f64::from(dims.width);
        let h = f64::from(dims.height);
        let div = |value: f64, by: f64| if by == 0.0 { 0.0 } else { value / by };
        Self(BoundingBox::new(
            Point::new(div(bbox.min.x, w), div(bbox.min.y, h)),
            Point::new(div(bbox.max.x, w), div(bbox.max.y, h)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denormalize_scales_each_axis() {
        let unit = UnitBoundingBox::new(0.1, 0.2, 0.5, 0.25);
        let px = unit.denormalize(Dimensions::new(1000, 800));
        assert_eq!(px.min, Point::new(100.0, 160.0));
        assert_eq!(px.width(), 500.0);
        assert_eq!(px.height(), 200.0);
    }

    #[test]
    fn normalize_is_inverse_of_denormalize() {
        let dims = Dimensions::new(1000, 800);
        let unit = UnitBoundingBox::new(0.1, 0.2, 0.5, 0.25);
        let round_trip = UnitBoundingBox::normalize(&unit.denormalize(dims), dims);
        assert_eq!(round_trip, unit);
    }
}
