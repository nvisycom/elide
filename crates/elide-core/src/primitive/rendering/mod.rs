//! Rendering primitives: visual attributes a redaction or render
//! instruction carries, independent of any one modality.

mod color;
mod dpi;
mod raster_mode;

pub use self::color::Color;
pub use self::dpi::Dpi;
pub use self::raster_mode::RasterMode;
