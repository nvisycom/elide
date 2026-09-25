//! Standalone value types the PDF codec is configured with: [`Dpi`] (a render
//! resolution) and [`RasterMode`] (whether and how to flatten pages to images).

mod dpi;
mod raster_mode;

pub use self::dpi::Dpi;
pub use self::raster_mode::RasterMode;
