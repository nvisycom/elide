//! The values the PDF codec is configured with: a render DPI and raster mode.
//!
//! [`Dpi`] is a render resolution, and [`RasterMode`] says whether and how to
//! flatten pages to images.

mod dpi;
mod raster_mode;

pub use self::dpi::Dpi;
pub use self::raster_mode::RasterMode;
