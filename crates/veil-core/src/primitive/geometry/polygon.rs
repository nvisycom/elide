//! A closed polygon.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Point;

/// A closed polygon, given by its ordered vertices.
///
/// A richer location than a [`BoundingBox`] for detections whose extent
/// is not rectangular — rotated text, a region traced by a vision model,
/// a signature. The boundary is implicitly closed: the last vertex
/// connects back to the first.
///
/// [`BoundingBox`]: super::BoundingBox
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Polygon(Vec<Point>);

impl Polygon {
    /// A polygon from its ordered vertices.
    pub fn new(vertices: impl Into<Vec<Point>>) -> Self {
        Self(vertices.into())
    }

    /// The polygon's vertices, in order.
    pub fn vertices(&self) -> &[Point] {
        &self.0
    }

    /// The number of vertices.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the polygon has no vertices.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<Point> for Polygon {
    fn from_iter<I: IntoIterator<Item = Point>>(vertices: I) -> Self {
        Self(vertices.into_iter().collect())
    }
}
