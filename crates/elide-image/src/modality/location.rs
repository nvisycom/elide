//! [`ImageLocation`]: a 2-D region within image content.

use std::cmp::Ordering;
use std::num::NonZeroU32;

use elide_core::modality::{ModalityLocation, Overlap};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::primitive::{BoundingBox, Point, Polygon};

/// A region of an image: a bounding box, optionally a tighter polygon.
///
/// An axis-aligned [`BoundingBox`] in pixel coordinates locates the
/// region; an optional [`Polygon`] captures a rotated or quadrilateral
/// shape when the source produced one (OCR engines that emit 4-point
/// polygons), and an optional page number addresses multi-page documents.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ImageLocation {
    /// Axis-aligned bounding box of the region, in pixel coordinates.
    pub bounding_box: BoundingBox<f64>,
    /// Polygon vertices when the region is rotated or quadrilateral.
    /// Axis-aligned-only sources leave this unset.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub polygon: Option<Polygon<f64>>,
    /// 1-based page number, for multi-page documents like PDFs.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub page: Option<NonZeroU32>,
}

impl ImageLocation {
    /// Location from the bounding box alone, every optional field unset.
    pub fn new(bounding_box: BoundingBox<f64>) -> Self {
        Self {
            bounding_box,
            polygon: None,
            page: None,
        }
    }

    /// Set the page number.
    #[must_use]
    pub fn with_page(mut self, page: NonZeroU32) -> Self {
        self.page = Some(page);
        self
    }

    /// Set the region polygon.
    #[must_use]
    pub fn with_polygon(mut self, polygon: Polygon<f64>) -> Self {
        self.polygon = Some(polygon);
        self
    }

    /// Region's shape as a polygon: its explicit [`polygon`] when set,
    /// otherwise its bounding box as a rectangle.
    ///
    /// [`polygon`]: Self::polygon
    fn shape(&self) -> Polygon<f64> {
        self.polygon
            .clone()
            .unwrap_or_else(|| self.bounding_box.to_polygon())
    }
}

impl ModalityLocation for ImageLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // Regions on different pages never overlap, even with identical
        // coordinates.
        if self.page != other.page {
            return Overlap::Disjoint;
        }
        // Exact shapes refine the bounding-box test: a rotated or
        // quadrilateral polygon is honored when present, so two boxes whose
        // polygons don't actually intersect aren't a false overlap.
        if !self.shape().overlaps(&other.shape()) {
            return Overlap::Disjoint;
        }
        // Containment and the IoU measure use the bounding box; exact
        // polygon containment is not modelled.
        self.bounding_box.overlap(&other.bounding_box)
    }

    fn covers(&self, other: &Self) -> bool {
        // Exact shapes, not bounding boxes: a triangle's box contains
        // regions that reach outside the triangle.
        self == other || (self.page == other.page && self.shape().contains(&other.shape()))
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single region can't span two pages; require agreement.
        if self.page != other.page {
            return None;
        }
        // The union of two boxes is an axis-aligned rectangle, so any
        // rotated/quadrilateral polygon is dropped, the bounding union is
        // what gets redacted.
        let mut location = Self::new(self.bounding_box.union(&other.bounding_box));
        if let Some(page) = self.page {
            location = location.with_page(page);
        }
        Some(location)
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // By area: the larger region is the more specific match.
        self.bounding_box
            .area()
            .total_cmp(&other.bounding_box.area())
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: page (an unpaged region first), then top-to-bottom,
        // then left-to-right.
        self.page
            .cmp(&other.page)
            .then(self.bounding_box.min.y.total_cmp(&other.bounding_box.min.y))
            .then(self.bounding_box.min.x.total_cmp(&other.bounding_box.min.x))
    }

    fn hash(&self) -> Vec<u8> {
        fn point(bytes: &mut Vec<u8>, p: &Point<f64>) {
            bytes.extend_from_slice(&p.x.to_bits().to_le_bytes());
            bytes.extend_from_slice(&p.y.to_bits().to_le_bytes());
        }
        let mut bytes = Vec::new();
        point(&mut bytes, &self.bounding_box.min);
        point(&mut bytes, &self.bounding_box.max);
        match &self.polygon {
            Some(polygon) => {
                bytes.push(1);
                bytes.extend_from_slice(&(polygon.vertices().len() as u64).to_le_bytes());
                for vertex in polygon.vertices() {
                    point(&mut bytes, vertex);
                }
            }
            None => bytes.push(0),
        }
        match self.page {
            Some(page) => {
                bytes.push(1);
                bytes.extend_from_slice(&page.get().to_le_bytes());
            }
            None => bytes.push(0),
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::{Dimensions, Point};

    fn page(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).unwrap()
    }

    /// A location over the box at `(x, y)` sized `w` by `h`.
    fn loc([x, y, w, h]: [f64; 4]) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    #[test]
    fn overlaps_requires_same_page() {
        let a = loc([0.0, 0.0, 10.0, 10.0]);
        let b = loc([5.0, 5.0, 10.0, 10.0]);
        assert!(a.overlaps(&b));
        let b_page2 = b.with_page(page(2));
        assert!(!a.overlaps(&b_page2));
    }

    #[test]
    fn a_polygon_covers_by_its_shape_not_its_box() {
        let triangle = loc([0.0, 0.0, 10.0, 10.0]).with_polygon(Polygon::new(vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(0.0, 10.0),
        ]));
        assert!(triangle.covers(&loc([1.0, 1.0, 2.0, 2.0])));
        assert!(!triangle.covers(&loc([7.0, 1.0, 2.0, 3.0])));
        assert!(loc([0.0, 0.0, 10.0, 10.0]).covers(&loc([7.0, 1.0, 2.0, 3.0])));
        assert!(triangle.covers(&triangle));
        assert!(
            !triangle
                .clone()
                .with_page(page(2))
                .covers(&loc([1.0, 1.0, 2.0, 2.0]))
        );
    }

    #[test]
    fn span_cmp_orders_by_area() {
        let small = loc([0.0, 0.0, 2.0, 2.0]);
        let large = loc([0.0, 0.0, 10.0, 10.0]);
        assert_eq!(small.span_cmp(&large), Ordering::Less);
    }

    #[test]
    fn position_cmp_is_reading_order() {
        let top = loc([50.0, 0.0, 5.0, 5.0]);
        let bottom_left = loc([0.0, 100.0, 5.0, 5.0]);
        // Top row sorts before a lower row regardless of x.
        assert_eq!(top.position_cmp(&bottom_left), Ordering::Less);
    }
}
