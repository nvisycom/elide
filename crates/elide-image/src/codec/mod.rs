//! Codec adapters: the raster formats (PNG, JPEG, TIFF) on the parts model.
//!
//! Each raster format decodes into a two-part [`Document`](elide_codec::Document):
//! the pixel [`Stream`](elide_codec::Stream) (the decoded image, redacted in
//! place and re-encoded) and the `#exif` [`Blob`](elide_codec::DocumentPart::Blob)
//! (the image's own bytes, re-read as the `Metadata` modality), recombined by
//! laying the redacted pixels over the metadata-stripped container. The three
//! formats differ only in their id and lookup keys (the `impl_image_handler!`
//! macro); the shared decode-redact-recompose body is `ImageDocumentLoader`,
//! split across `image_loader` (decode into parts), `image_state` (the shared
//! decoded buffer), `image_stream` (the pixel body part), and `image_recombine`
//! (folding the redacted pixels over the `#exif` blob).

mod macros;

mod exif_handler;
mod image_loader;
mod image_recombine;
mod image_state;
mod image_stream;
#[cfg(feature = "jpeg")]
mod jpeg_handler;
#[cfg(feature = "png")]
mod png_handler;
#[cfg(feature = "tiff")]
mod tiff_handler;

/// The `#exif` sub-part id: the image's own bytes, re-read as `Metadata`.
const EXIF_PART_ID: &str = "#exif";

/// The pixel body part id: the decoded (redacted) image.
const PIXEL_PART_ID: &str = "pixels";

pub use self::exif_handler::format as exif_format;
#[cfg(feature = "jpeg")]
pub use self::jpeg_handler::{format as jpeg_format, format_with as jpeg_format_with};
#[cfg(feature = "png")]
pub use self::png_handler::{format as png_format, format_with as png_format_with};
#[cfg(feature = "tiff")]
pub use self::tiff_handler::{format as tiff_format, format_with as tiff_format_with};
