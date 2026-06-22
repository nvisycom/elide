//! [`ImageLocation`]: a 2-D region within image content.

use std::cmp::Ordering;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityLocation;
use crate::primitive::{BoundingBox, Polygon};

/// Region within image content.
///
/// An axis-aligned [`BoundingBox`] in pixel coordinates locates the
/// region; an optional [`Polygon`] captures a rotated or quadrilateral
/// shape when the source produced one (OCR engines that emit 4-point
/// polygons), and an optional page number addresses multi-page documents.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ImageLocation {
    /// Axis-aligned bounding box of the region, in pixel coordinates.
    pub bounding_box: BoundingBox,
    /// Polygon vertices when the region is rotated or quadrilateral.
    /// Axis-aligned-only sources leave this unset.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub polygon: Option<Polygon>,
    /// 1-based page number, for multi-page documents like PDFs.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub page: Option<u32>,
}

impl ImageLocation {
    /// Location from the bounding box alone, every optional field unset.
    pub fn new(bounding_box: BoundingBox) -> Self {
        Self {
            bounding_box,
            polygon: None,
            page: None,
        }
    }

    /// Set the page number.
    #[must_use]
    pub fn with_page(mut self, page: u32) -> Self {
        self.page = Some(page);
        self
    }

    /// Set the region polygon.
    #[must_use]
    pub fn with_polygon(mut self, polygon: Polygon) -> Self {
        self.polygon = Some(polygon);
        self
    }

    /// Region's shape as a polygon: its explicit [`polygon`] when set,
    /// otherwise its bounding box as a rectangle.
    ///
    /// [`polygon`]: Self::polygon
    fn shape(&self) -> Polygon {
        self.polygon
            .clone()
            .unwrap_or_else(|| self.bounding_box.to_polygon())
    }
}

impl ModalityLocation for ImageLocation {
    fn overlaps(&self, other: &Self) -> bool {
        // Regions on different pages never overlap, even with identical
        // coordinates. On the same page, compare exact shapes: a rotated
        // or quadrilateral polygon is honored when present, so two boxes
        // whose polygons don't actually intersect aren't false positives.
        if self.page != other.page {
            return false;
        }
        // Bounding-box test first as a cheap reject; polygons only narrow
        // it, so a box miss is a definite miss.
        self.bounding_box.overlaps(&other.bounding_box) && self.shape().overlaps(&other.shape())
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // By area: the larger region is the more specific match.
        self.bounding_box
            .area()
            .total_cmp(&other.bounding_box.area())
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: page, then top-to-bottom, then left-to-right.
        self.page
            .unwrap_or(0)
            .cmp(&other.page.unwrap_or(0))
            .then(self.bounding_box.min.y.total_cmp(&other.bounding_box.min.y))
            .then(self.bounding_box.min.x.total_cmp(&other.bounding_box.min.x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::Point;

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    #[test]
    fn overlaps_requires_same_page() {
        let a = loc(0.0, 0.0, 10.0, 10.0);
        let b = loc(5.0, 5.0, 10.0, 10.0);
        assert!(a.overlaps(&b));
        let b_page2 = b.clone().with_page(2);
        assert!(!a.overlaps(&b_page2));
    }

    #[test]
    fn span_cmp_orders_by_area() {
        let small = loc(0.0, 0.0, 2.0, 2.0);
        let large = loc(0.0, 0.0, 10.0, 10.0);
        assert_eq!(small.span_cmp(&large), Ordering::Less);
    }

    #[test]
    fn position_cmp_is_reading_order() {
        let top = loc(50.0, 0.0, 5.0, 5.0);
        let bottom_left = loc(0.0, 100.0, 5.0, 5.0);
        // Top row sorts before a lower row regardless of x.
        assert_eq!(top.position_cmp(&bottom_left), Ordering::Less);
    }
}
