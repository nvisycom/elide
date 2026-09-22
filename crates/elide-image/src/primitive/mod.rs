//! Image primitives: the 2-D geometry an image location is built from, and the
//! [`Color`] a redaction paints.
//!
//! [`Point`], an axis-aligned [`BoundingBox`] (pixel coordinates), its normalized
//! `0.0..=1.0` form [`UnitBoundingBox`], a closed [`Polygon`], and the pixel
//! [`Dimensions`] that convert between normalized and pixel space, plus the
//! [`PixelRegion`] a crop or fill addresses. These are the shapes; binding one to
//! the modality (as [`ImageLocation`](crate::modality::ImageLocation)) is the
//! modality's job.

mod bounding_box;
mod color;
mod dimensions;
mod pixel_region;
mod polygon;
mod unit_bounding_box;

pub use self::bounding_box::{BoundingBox, Point};
pub use self::color::Color;
pub use self::dimensions::Dimensions;
pub use self::pixel_region::PixelRegion;
pub use self::polygon::Polygon;
pub use self::unit_bounding_box::UnitBoundingBox;
