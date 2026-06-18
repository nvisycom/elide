//! A 2-D point and the axis-aligned bounding box built from it.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A point in a 2-D coordinate space.
///
/// The coordinate basis is left to the consumer: pixel coordinates for a
/// raster image, normalized `0.0..=1.0` coordinates for a
/// resolution-independent region, or page units for a document. The
/// model only requires the two scalars.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Point {
    /// The horizontal coordinate.
    pub x: f64,
    /// The vertical coordinate.
    pub y: f64,
}

impl Point {
    /// A point at `(x, y)`.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle, given by its minimum and maximum corners.
///
/// The location type for the image and document modalities: where a
/// detected entity sits within a rendered page. [`min`] is the top-left
/// corner and [`max`] the bottom-right under the usual screen convention
/// (y grows downward), though the box itself is agnostic to coordinate
/// orientation.
///
/// [`min`]: Self::min
/// [`max`]: Self::max
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BoundingBox {
    /// The minimum corner (top-left, conventionally).
    pub min: Point,
    /// The maximum corner (bottom-right, conventionally).
    pub max: Point,
}

impl BoundingBox {
    /// A box spanning the two corners.
    pub const fn new(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    /// A box from a top-left origin and a size.
    pub fn from_origin_size(origin: Point, width: f64, height: f64) -> Self {
        Self {
            min: origin,
            max: Point::new(origin.x + width, origin.y + height),
        }
    }

    /// The box width (`max.x - min.x`).
    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    /// The box height (`max.y - min.y`).
    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    /// The box area (`width * height`).
    pub fn area(&self) -> f64 {
        self.width() * self.height()
    }

    /// Whether this box overlaps `other`.
    ///
    /// Rectangle intersection: the boxes share interior area. Touching
    /// edges alone do not count as overlapping.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.min.x < other.max.x
            && other.min.x < self.max.x
            && self.min.y < other.max.y
            && other.min.y < self.max.y
    }
}
