//! Closed polygon.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Point;

/// Closed polygon, given by its ordered vertices.
///
/// A richer location than a [`BoundingBox`] for detections whose extent
/// is not rectangular: rotated text, a region traced by a vision model, a
/// signature. The boundary is implicitly closed, so the last vertex
/// connects back to the first.
///
/// [`BoundingBox`]: super::BoundingBox
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Polygon(Vec<Point>);

impl Polygon {
    /// Polygon from its ordered vertices.
    pub fn new(vertices: impl Into<Vec<Point>>) -> Self {
        Self(vertices.into())
    }

    /// Polygon's vertices, in order.
    pub fn vertices(&self) -> &[Point] {
        &self.0
    }

    /// Number of vertices.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the polygon has no vertices.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether this polygon overlaps `other`.
    ///
    /// Uses the separating-axis theorem, which is exact for **convex**
    /// polygons (axis-aligned or rotated rectangles, quadrilaterals from
    /// an OCR engine, vision-model regions). Two polygons overlap when no
    /// edge-normal axis separates them. A polygon with fewer than three
    /// vertices encloses no area and never overlaps.
    ///
    /// Touching-but-disjoint polygons (sharing only an edge or vertex) do
    /// not count as overlapping, matching the
    /// [`BoundingBox`](super::BoundingBox) convention.
    pub fn overlaps(&self, other: &Self) -> bool {
        if self.0.len() < 3 || other.0.len() < 3 {
            return false;
        }
        !has_separating_axis(&self.0, &other.0) && !has_separating_axis(&other.0, &self.0)
    }
}

/// Whether any edge of `a` yields an axis that separates `a` from `b`
/// (their projections onto the edge normal don't overlap).
fn has_separating_axis(a: &[Point], b: &[Point]) -> bool {
    for i in 0..a.len() {
        let edge = a[(i + 1) % a.len()] - a[i];
        let axis = edge.perp();
        let (a_min, a_max) = project(a, axis);
        let (b_min, b_max) = project(b, axis);
        // A strict gap means a separating axis exists (touching is
        // disjoint).
        if a_max <= b_min || b_max <= a_min {
            return true;
        }
    }
    false
}

/// Project `points` onto `axis`, returning the `(min, max)` of the dot
/// products.
fn project(points: &[Point], axis: Point) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &p in points {
        let dot = p.dot(axis);
        min = min.min(dot);
        max = max.max(dot);
    }
    (min, max)
}

impl FromIterator<Point> for Polygon {
    fn from_iter<I: IntoIterator<Item = Point>>(vertices: I) -> Self {
        Self(vertices.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Axis-aligned square `[x, x+s] x [y, y+s]` as a polygon.
    fn square(x: f64, y: f64, s: f64) -> Polygon {
        Polygon::new(vec![
            Point::new(x, y),
            Point::new(x + s, y),
            Point::new(x + s, y + s),
            Point::new(x, y + s),
        ])
    }

    #[test]
    fn overlapping_squares() {
        assert!(square(0.0, 0.0, 10.0).overlaps(&square(5.0, 5.0, 10.0)));
    }

    #[test]
    fn disjoint_squares() {
        assert!(!square(0.0, 0.0, 10.0).overlaps(&square(20.0, 20.0, 5.0)));
    }

    #[test]
    fn touching_edges_do_not_overlap() {
        // Right edge of the first meets the left edge of the second.
        assert!(!square(0.0, 0.0, 10.0).overlaps(&square(10.0, 0.0, 10.0)));
    }

    #[test]
    fn rotated_diamond_overlaps_square() {
        // A diamond centered at (5, 5) overlapping a unit square at origin.
        let diamond = Polygon::new(vec![
            Point::new(5.0, 0.0),
            Point::new(10.0, 5.0),
            Point::new(5.0, 10.0),
            Point::new(0.0, 5.0),
        ]);
        assert!(diamond.overlaps(&square(0.0, 0.0, 3.0)));
    }

    #[test]
    fn degenerate_polygon_never_overlaps() {
        let line = Polygon::new(vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)]);
        assert!(!line.overlaps(&square(0.0, 0.0, 10.0)));
    }
}
