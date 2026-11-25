//! Closed polygon.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Coordinate, Point};

/// A closed polygon, given by its ordered vertices.
///
/// Generic over the coordinate scalar (a [`Coordinate`]).
///
/// A richer location than a [`BoundingBox`] for detections whose extent
/// is not rectangular: rotated text, a region traced by a vision model, a
/// signature. The boundary is implicitly closed, so the last vertex
/// connects back to the first.
///
/// Like [`BoundingBox`], `Polygon<f64>` is a fractional geometric claim; the
/// overlap test ([`overlaps`]) lives only on that float instantiation, as it is
/// pure floating-point geometry.
///
/// [`BoundingBox`]: super::BoundingBox
/// [`overlaps`]: Self::overlaps
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Polygon<C: Coordinate>(Vec<Point<C>>);

#[cfg(feature = "schema")]
super::schema::coordinate_transparent_schema!(Polygon(Vec<Point<C>>));

impl<C: Coordinate> Polygon<C> {
    /// Polygon from its ordered vertices.
    pub fn new(vertices: impl Into<Vec<Point<C>>>) -> Self {
        Self(vertices.into())
    }

    /// Polygon's vertices, in order.
    pub fn vertices(&self) -> &[Point<C>] {
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
}

impl Polygon<f64> {
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
    /// [`BoundingBox`] convention.
    ///
    /// [`BoundingBox`]: super::BoundingBox
    pub fn overlaps(&self, other: &Self) -> bool {
        if self.0.len() < 3 || other.0.len() < 3 {
            return false;
        }
        !self.separates(other) && !other.separates(self)
    }

    /// Whether this polygon contains all of `other`, boundary included.
    ///
    /// Every vertex of `other` must lie on the inner side of every edge of
    /// `self`. For a **convex** polygon that is exact, as `other` lies within
    /// the hull of its vertices. For a concave one the test admits only what
    /// lies within every edge, a subset of the polygon, so it can deny a
    /// containment but never claim a false one. A polygon enclosing no area
    /// contains nothing, and nothing contains an empty polygon.
    #[must_use]
    pub fn contains(&self, other: &Self) -> bool {
        let n = self.0.len();
        if n < 3 || other.0.is_empty() {
            return false;
        }
        // Twice the signed area: its sign gives the winding, and so which
        // side of each edge is inside.
        let winding: f64 = (0..n)
            .map(|i| self.0[i].perp().dot(self.0[(i + 1) % n]))
            .sum();
        if winding == 0.0 {
            return false;
        }
        (0..n).all(|i| {
            let start = self.0[i];
            let normal = (self.0[(i + 1) % n] - start).perp();
            other
                .0
                .iter()
                .all(|&p| normal.dot(p - start) * winding.signum() >= 0.0)
        })
    }

    /// Whether any edge of `self` yields an axis that separates it from
    /// `other` (their projections onto that edge normal don't overlap).
    fn separates(&self, other: &Self) -> bool {
        for i in 0..self.0.len() {
            let edge = self.0[(i + 1) % self.0.len()] - self.0[i];
            let axis = edge.perp();
            let (a_min, a_max) = self.project(axis);
            let (b_min, b_max) = other.project(axis);
            // A strict gap means a separating axis exists (touching is
            // disjoint).
            if a_max <= b_min || b_max <= a_min {
                return true;
            }
        }
        false
    }

    /// Project the vertices onto `axis`, returning the `(min, max)` of the dot
    /// products.
    fn project(&self, axis: Point<f64>) -> (f64, f64) {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &p in &self.0 {
            let d = p.dot(axis);
            min = min.min(d);
            max = max.max(d);
        }
        (min, max)
    }
}

impl<C: Coordinate> FromIterator<Point<C>> for Polygon<C> {
    fn from_iter<I: IntoIterator<Item = Point<C>>>(vertices: I) -> Self {
        Self(vertices.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Axis-aligned square `[x, x+s] x [y, y+s]` as a polygon.
    fn square(x: f64, y: f64, s: f64) -> Polygon<f64> {
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
    fn a_triangle_contains_only_what_lies_within_it() {
        let triangle = Polygon::new(vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(0.0, 10.0),
        ]);
        let inside = Polygon::new(vec![
            Point::new(1.0, 1.0),
            Point::new(3.0, 1.0),
            Point::new(3.0, 3.0),
            Point::new(1.0, 3.0),
        ]);
        let corner = Polygon::new(vec![
            Point::new(7.0, 1.0),
            Point::new(9.0, 1.0),
            Point::new(9.0, 4.0),
            Point::new(7.0, 4.0),
        ]);
        assert!(triangle.contains(&inside));
        assert!(!triangle.contains(&corner));
        assert!(triangle.contains(&triangle));
    }

    #[test]
    fn containment_holds_for_either_winding() {
        let clockwise = Polygon::new(
            square(0.0, 0.0, 10.0)
                .vertices()
                .iter()
                .rev()
                .copied()
                .collect::<Vec<_>>(),
        );
        assert!(clockwise.contains(&square(2.0, 2.0, 3.0)));
        assert!(!clockwise.contains(&square(8.0, 8.0, 3.0)));
    }

    #[test]
    fn degenerate_polygon_never_overlaps() {
        let line = Polygon::new(vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)]);
        assert!(!line.overlaps(&square(0.0, 0.0, 10.0)));
    }
}
