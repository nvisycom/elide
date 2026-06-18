//! A 2-D point and the axis-aligned bounding box built from it.

use std::ops::Sub;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Polygon;

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

    /// The dot product of `self` and `other` as vectors.
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// The left perpendicular of `self` as a vector: `(-y, x)`.
    ///
    /// Rotating a vector 90 degrees counter-clockwise. Used to turn an
    /// edge direction into the axis normal to it.
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }
}

impl Sub for Point {
    type Output = Self;

    /// Component-wise subtraction, treating both points as position
    /// vectors: the displacement (edge) vector from `rhs` to `self`.
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
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

    /// The box as a four-vertex [`Polygon`] (clockwise from the top-left
    /// corner under the usual screen convention).
    ///
    /// Lets a box be compared against a rotated or quadrilateral region
    /// through [`Polygon::overlaps`].
    pub fn to_polygon(&self) -> Polygon {
        Polygon::new(vec![
            self.min,
            Point::new(self.max.x, self.min.y),
            self.max,
            Point::new(self.min.x, self.max.y),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_vector_ops() {
        let a = Point::new(3.0, 4.0);
        let b = Point::new(1.0, 2.0);
        assert_eq!(a - b, Point::new(2.0, 2.0));
        assert_eq!(a.dot(b), 11.0);
        assert_eq!(a.perp(), Point::new(-4.0, 3.0));
    }

    #[test]
    fn overlaps_and_area() {
        let a = BoundingBox::from_origin_size(Point::new(0.0, 0.0), 10.0, 10.0);
        let b = BoundingBox::from_origin_size(Point::new(5.0, 5.0), 10.0, 10.0);
        assert!(a.overlaps(&b));
        assert_eq!(a.area(), 100.0);
        // Touching edge only: not an overlap.
        let c = BoundingBox::from_origin_size(Point::new(10.0, 0.0), 5.0, 5.0);
        assert!(!a.overlaps(&c));
    }
}
