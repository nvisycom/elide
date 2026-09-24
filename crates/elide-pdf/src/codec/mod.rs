//! Codec adapter: binds this crate's PDF engine to the `elide-codec`
//! [`Handler`](elide_codec::Handler)/[`Loader`](elide_codec::Loader) contracts.
//!
//! [`pdf_format`] decodes on the glyph-deletion redaction path (the default,
//! pure-Rust). With the `render` feature, `pdf_format_with` builds the
//! format with an explicit [`RasterMode`]: `RasterMode::Always` flattens
//! every page to an image instead.

mod dpi;
mod pdf_handler;
mod pdf_loader;
mod raster_mode;

pub use self::dpi::Dpi;
pub use self::pdf_handler::format as pdf_format;
#[cfg(feature = "render")]
pub use self::pdf_handler::format_with as pdf_format_with;
pub(crate) use self::pdf_loader::PdfLoader;
pub use self::raster_mode::RasterMode;
