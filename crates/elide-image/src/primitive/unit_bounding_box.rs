//! Pixel-space bridges for [`UnitBoundingBox`], the normalized `0.0..=1.0` box.
//!
//! The box itself lives in `elide-core` (a size-agnostic value). Image code
//! needs to cross between it and pixel-space [`BoundingBox`]es, so
//! [`UnitBoundingBoxExt`] adds that bridge here, where the pixel
//! [`Dimensions`] it scales against live.

use elide_core::primitive::UnitBoundingBox;

use super::{BoundingBox, Dimensions, Point};

/// Pixel-space conversions for a [`UnitBoundingBox`].
pub trait UnitBoundingBoxExt {
    /// Scale into a pixel-space [`BoundingBox`] for an image of `dims`,
    /// multiplying each axis by the matching dimension.
    fn denormalize(&self, dims: Dimensions<u32>) -> BoundingBox<f64>;

    /// Build a unit box from a pixel-space [`BoundingBox`] on an image of
    /// `dims`, dividing each axis by the matching dimension.
    ///
    /// The inverse of [`denormalize`](Self::denormalize). A zero `dims` axis
    /// yields a zero on that axis rather than a non-finite value.
    fn normalize(bbox: &BoundingBox<f64>, dims: Dimensions<u32>) -> Self;
}

impl UnitBoundingBoxExt for UnitBoundingBox {
    fn denormalize(&self, dims: Dimensions<u32>) -> BoundingBox<f64> {
        let w = f64::from(dims.width);
        let h = f64::from(dims.height);
        BoundingBox::new(
            Point::new(self.x * w, self.y * h),
            Point::new((self.x + self.width) * w, (self.y + self.height) * h),
        )
    }

    fn normalize(bbox: &BoundingBox<f64>, dims: Dimensions<u32>) -> Self {
        let w = f64::from(dims.width);
        let h = f64::from(dims.height);
        let div = |value: f64, by: f64| if by == 0.0 { 0.0 } else { value / by };
        UnitBoundingBox::new(
            div(bbox.min.x, w),
            div(bbox.min.y, h),
            div(bbox.width(), w),
            div(bbox.height(), h),
        )
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
        let round_trip =
            <UnitBoundingBox as UnitBoundingBoxExt>::normalize(&unit.denormalize(dims), dims);
        assert_eq!(round_trip, unit);
    }
}
