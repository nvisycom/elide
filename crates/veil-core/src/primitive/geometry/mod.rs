//! 2-D geometry primitives for spatial spans.
//!
//! The building blocks a visual modality crate uses to define *where*
//! an entity sits on a rendered page: a [`Point`], an axis-aligned
//! [`BoundingBox`], or a closed [`Polygon`]. The core only supplies the
//! shapes; binding one to a modality (as that modality's `Span`) is the
//! modality crate's job.

mod bounding_box;
mod polygon;

pub use self::bounding_box::{BoundingBox, Point};
pub use self::polygon::Polygon;
