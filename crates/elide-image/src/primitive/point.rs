//! [`Point`]: a 2-D coordinate pair, generic over the coordinate scalar.

use core::ops::Sub;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A scalar a [`Point`] coordinate can hold.
///
/// A sealed, closed set (the numeric primitives this crate's geometry uses),
/// so a `Point` cannot be built over an arbitrary type. Modelled on the way
/// `core`'s `NonZero<T>` constrains its parameter to a fixed set of primitives
/// rather than an open arithmetic bound.
pub trait Coordinate: sealed::Sealed + Copy + PartialEq + PartialOrd {
    /// A `PascalCase` tag for this scalar (`U32`, `F64`), appended to a generic
    /// geometry primitive's JSON-schema name so `BoundingBox<u32>` and
    /// `BoundingBox<f64>` get the distinct, stable names `BoundingBoxU32` and
    /// `BoundingBoxF64` rather than colliding on `BoundingBox`.
    #[cfg(feature = "schema")]
    const SCHEMA_SUFFIX: &'static str;

    /// `origin + size`, the far corner of a box spanning `size` from this
    /// origin. Integer coordinates saturate at their maximum (a far-off origin
    /// plus a large size stays representable, giving a box that clamps to
    /// nothing rather than overflowing); floating-point coordinates add plainly.
    #[must_use]
    fn advance(self, size: Self) -> Self;
}

mod sealed {
    pub trait Sealed {}
}

macro_rules! impl_coordinate {
    (float: $($t:ty => $suffix:literal),+ $(,)?) => {
        $(
            impl sealed::Sealed for $t {}
            impl Coordinate for $t {
                #[cfg(feature = "schema")]
                const SCHEMA_SUFFIX: &'static str = $suffix;
                fn advance(self, size: Self) -> Self {
                    self + size
                }
            }
        )+
    };
    (int: $($t:ty => $suffix:literal),+ $(,)?) => {
        $(
            impl sealed::Sealed for $t {}
            impl Coordinate for $t {
                #[cfg(feature = "schema")]
                const SCHEMA_SUFFIX: &'static str = $suffix;
                fn advance(self, size: Self) -> Self {
                    self.saturating_add(size)
                }
            }
        )+
    };
}

// The coordinate scalars the crate's geometry speaks in: integer pixel
// coordinates (`u32`), signed offsets (`i32`), and floating-point claims
// (`f32`/`f64`).
impl_coordinate!(int: u32 => "U32", i32 => "I32");
impl_coordinate!(float: f32 => "F32", f64 => "F64");

/// A point in a 2-D coordinate space.
///
/// The coordinate basis is left to the consumer: pixel coordinates for a raster
/// image, normalized `0.0..=1.0` coordinates for a resolution-independent
/// region, or page units for a document. The scalar type is the [`Coordinate`]
/// parameter, `Point<f64>` for a fractional geometric claim, `Point<u32>` for an
/// integer pixel position.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Point<T: Coordinate> {
    /// Horizontal coordinate.
    pub x: T,
    /// Vertical coordinate.
    pub y: T,
}

#[cfg(feature = "schema")]
super::schema::coordinate_object_schema!(Point {
    x: C = "Horizontal coordinate.",
    y: C = "Vertical coordinate."
});

impl<T: Coordinate> Point<T> {
    /// Point at `(x, y)`.
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

impl Point<f64> {
    /// The left perpendicular `(-y, x)`: this vector rotated 90 degrees
    /// counter-clockwise, turning an edge direction into the axis normal to it.
    #[must_use]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Dot product of `self` and `other`, read as vectors.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }
}

/// Component-wise difference, reading the two points as position vectors: the
/// displacement vector from `rhs` to `self`.
impl Sub for Point<f64> {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}
