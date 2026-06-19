//! Axis-aligned bounding box in unit-square `0.0..=1.0` coordinates.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{BoundingBox, Dimensions, Point};

/// Axis-aligned bounding box in normalized `0.0..=1.0` coordinates.
///
/// Used at API boundaries where the producer doesn't know the image's
/// pixel size, most commonly the output of vision-language models that
/// never see the original resolution. `(0, 0)` is the top-left corner of
/// the image and `(1, 1)` the bottom-right.
///
/// Field values are not clamped to `0.0..=1.0`; the type carries the
/// *intent* of unit-square coordinates, not a hard invariant. Conversion
/// to pixel space with [`denormalize`](Self::denormalize) is mechanical
/// multiplication regardless, so out-of-range input yields out-of-range
/// pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct UnitBoundingBox {
    /// Top-left x in `0.0..=1.0` (fraction of image width).
    pub x: f64,
    /// Top-left y in `0.0..=1.0` (fraction of image height).
    pub y: f64,
    /// Width in `0.0..=1.0` (fraction of image width).
    pub width: f64,
    /// Height in `0.0..=1.0` (fraction of image height).
    pub height: f64,
}

impl UnitBoundingBox {
    /// Normalized box from explicit fields.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Scale into a pixel-space [`BoundingBox`] for an image of `dims`,
    /// multiplying each axis by the matching dimension.
    pub fn denormalize(&self, dims: Dimensions) -> BoundingBox {
        let w = f64::from(dims.width);
        let h = f64::from(dims.height);
        BoundingBox::from_origin_size(
            Point::new(self.x * w, self.y * h),
            self.width * w,
            self.height * h,
        )
    }

    /// Build a unit box from a pixel-space [`BoundingBox`] on an image of
    /// `dims`, dividing each axis by the matching dimension.
    ///
    /// The inverse of [`denormalize`](Self::denormalize). A zero `dims`
    /// axis yields a zero on that axis rather than a non-finite value.
    pub fn normalize(bbox: &BoundingBox, dims: Dimensions) -> Self {
        let w = f64::from(dims.width);
        let h = f64::from(dims.height);
        let div = |value: f64, by: f64| if by == 0.0 { 0.0 } else { value / by };
        Self {
            x: div(bbox.min.x, w),
            y: div(bbox.min.y, h),
            width: div(bbox.width(), w),
            height: div(bbox.height(), h),
        }
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
