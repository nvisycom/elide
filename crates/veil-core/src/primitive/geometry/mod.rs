//! 2-D geometry primitives for spatial spans.
//!
//! The building blocks a visual modality uses to define *where* an entity
//! sits on a rendered page: a [`Point`], an axis-aligned [`BoundingBox`]
//! (pixel coordinates), its normalized `0.0..=1.0` form
//! [`UnitBoundingBox`], a closed [`Polygon`], and the pixel
//! [`Dimensions`] that convert between normalized and pixel space. The
//! core only supplies the shapes; binding one to a modality (as that
//! modality's location type) is the modality's job.

mod bounding_box;
mod dimensions;
mod polygon;
mod unit_bounding_box;

pub use self::bounding_box::{BoundingBox, Point};
pub use self::dimensions::Dimensions;
pub use self::polygon::Polygon;
pub use self::unit_bounding_box::UnitBoundingBox;
