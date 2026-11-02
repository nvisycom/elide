//! Codec adapters: the raster formats (PNG, JPEG, TIFF) on the parts model.
//!
//! Each raster format decodes into a [`Document`](elide_codec::Document): the
//! pixel [`Stream`](elide_codec::Stream) (the decoded image, redacted in place
//! and re-encoded) and, when the hidden-content policy treats metadata as
//! content, the `#exif` stream (the surfaced EXIF fields, as the `Metadata`
//! modality). Both edit one shared buffer, which the recombiner encodes once.
//! The three formats differ only in their id and lookup keys (the
//! `impl_image_handler!` macro); the shared decode-redact-recompose body is
//! `ImageDocumentLoader`, split across `image_loader` (decode into parts),
//! `image_state` (the shared decoded buffer), `image_stream` (the pixel body
//! part), `exif_stream` (the metadata part), and `image_recombine` (encoding
//! the buffer).

mod macros;

mod exif_stream;
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

/// The `#exif` part id: the image's surfaced EXIF fields, as `Metadata`.
const EXIF_PART_ID: &str = "#exif";

/// The pixel body part id: the decoded (redacted) image.
const PIXEL_PART_ID: &str = "pixels";

#[cfg(feature = "jpeg")]
pub use self::jpeg_handler::format as jpeg_format;
#[cfg(feature = "png")]
pub use self::png_handler::format as png_format;
#[cfg(feature = "tiff")]
pub use self::tiff_handler::format as tiff_format;
