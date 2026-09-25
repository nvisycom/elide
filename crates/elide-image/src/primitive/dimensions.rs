//! [`Dimensions`]: a width and a height over a coordinate scalar.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Coordinate;

/// A `width` x `height` size, generic over the coordinate scalar (a
/// [`Coordinate`]).
///
/// `Dimensions<u32>` is an image or canvas size in whole pixels;
/// `Dimensions<f64>` a fractional extent. Paired with a [`Point`](super::Point)
/// origin, it constructs a [`BoundingBox`](super::BoundingBox). It also converts
/// between normalized `0.0..=1.0` coordinates (what vision models typically emit)
/// and absolute pixels; see
/// [`UnitBoundingBox::denormalize`](super::UnitBoundingBox::denormalize).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Dimensions<C: Coordinate> {
    /// Width.
    pub width: C,
    /// Height.
    pub height: C,
}

#[cfg(feature = "schema")]
super::schema::coordinate_object_schema!(Dimensions {
    width: C = "Width.",
    height: C = "Height.",
});

impl<C: Coordinate> Dimensions<C> {
    /// Dimensions from an explicit width and height.
    pub const fn new(width: C, height: C) -> Self {
        Self { width, height }
    }
}

// `Eq` where the scalar is: an integer `Dimensions` (e.g. `Dimensions<u32>`) is a
// total equality, so it can sit inside `Eq` types like `ImageData`. The derive
// cannot express the `C: Eq` bound (a `Coordinate` need not be `Eq` — `f64` is
// not), so this is written by hand; the float dimensions simply lack `Eq`.
impl<C: Coordinate + Eq> Eq for Dimensions<C> {}

impl<C: Coordinate> From<(C, C)> for Dimensions<C> {
    fn from((width, height): (C, C)) -> Self {
        Self { width, height }
    }
}
