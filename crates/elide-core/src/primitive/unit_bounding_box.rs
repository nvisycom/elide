//! [`UnitBoundingBox`]: an axis-aligned box in normalized `0.0..=1.0`
//! coordinates.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box in normalized `0.0..=1.0` coordinates, top-left
/// origin: `(0, 0)` is the top-left corner of the medium and `(1, 1)` the
/// bottom-right, with `y` increasing downward.
///
/// It names a position *within* a rendered medium without knowing its pixel
/// size, so one value serves a viewer at any zoom: a highlight over a PDF page,
/// a region a vision-language model reports, a box on an image. The producer
/// divides device coordinates by the medium's dimensions; the consumer
/// multiplies back by whatever size it renders at.
///
/// Stored and serialized as the flat `{ x, y, width, height }` a renderer wants
/// (`x`/`y` the top-left origin, both in fractions). Field values are not clamped
/// to `0.0..=1.0`: the type carries the *intent* of unit-square coordinates, not
/// a hard invariant, and a box may legitimately sit slightly outside the medium.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UnitBoundingBox {
    /// Left edge, as a fraction of the medium width (top-left origin).
    pub x: f64,
    /// Top edge, as a fraction of the medium height (`y` increases downward).
    pub y: f64,
    /// Width, as a fraction of the medium width.
    pub width: f64,
    /// Height, as a fraction of the medium height.
    pub height: f64,
}

impl UnitBoundingBox {
    /// A normalized box from a top-left origin `(x, y)` and a size, all in
    /// `0.0..=1.0` fractions.
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The smallest box covering both `self` and `other` (their union), the
    /// operation that merges per-glyph boxes into one box per line.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self::new(x, y, right - x, bottom - y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_covers_both_boxes() {
        let a = UnitBoundingBox::new(0.1, 0.2, 0.2, 0.1);
        let b = UnitBoundingBox::new(0.4, 0.2, 0.2, 0.1);
        let u = a.union(b);
        let close = |x: f64, y: f64| (x - y).abs() < 1e-9;
        assert!(close(u.x, 0.1) && close(u.y, 0.2), "origin: {u:?}");
        assert!(close(u.width, 0.5) && close(u.height, 0.1), "size: {u:?}");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serializes_as_flat_origin_size() {
        let unit = UnitBoundingBox::new(0.1, 0.2, 0.5, 0.25);
        let json = serde_json::to_value(unit).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({ "x": 0.1, "y": 0.2, "width": 0.5, "height": 0.25 })
        );
        let back: UnitBoundingBox = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, unit);
    }
}
