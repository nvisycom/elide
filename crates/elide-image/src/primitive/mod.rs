//! Image primitives: the 2-D geometry an image location is built from, and the
//! [`Color`] a redaction paints.
//!
//! A [`Point`] and the axis-aligned [`BoundingBox`] built from two of them,
//! generic over the coordinate scalar: `BoundingBox<f64>` is a fractional
//! geometric claim, `BoundingBox<u32>` the concrete pixels a crop or fill
//! addresses. Its normalized `0.0..=1.0` form is [`UnitBoundingBox`], its
//! rotated form a closed [`Polygon`], and [`Dimensions`] convert between
//! normalized and pixel space. These are the shapes; binding one to the modality
//! (as [`ImageLocation`](crate::modality::ImageLocation)) is the modality's job.

mod bounding_box;
mod color;
mod dimensions;
mod point;
mod polygon;
mod unit_bounding_box;

pub use self::bounding_box::BoundingBox;
pub use self::color::Color;
pub use self::dimensions::Dimensions;
pub use self::point::{Coordinate, Point};
pub use self::polygon::Polygon;
pub use self::unit_bounding_box::UnitBoundingBox;
