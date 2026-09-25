//! Codec adapters: the raster formats (PNG, JPEG, TIFF) on the parts model.
//!
//! Each raster format decodes into a two-part [`Document`](elide_codec::Document):
//! the pixel [`Stream`](elide_codec::Stream) (the decoded image, redacted in
//! place and re-encoded) and the `#exif` [`Blob`](elide_codec::DocumentPart::Blob)
//! (the image's own bytes, re-read as the `Metadata` modality), recombined by
//! laying the redacted pixels over the metadata-stripped container. The three
//! formats differ only in their id and lookup keys (the `impl_image_handler!`
//! macro); the shared decode-redact-recompose body is `document`'s
//! `ImageDocumentLoader`.

mod macros;

mod document;
mod exif_handler;
#[cfg(feature = "jpeg")]
mod jpeg_handler;
#[cfg(feature = "png")]
mod png_handler;
#[cfg(feature = "tiff")]
mod tiff_handler;

pub use self::exif_handler::format as exif_format;
#[cfg(feature = "jpeg")]
pub use self::jpeg_handler::{format as jpeg_format, format_with as jpeg_format_with};
#[cfg(feature = "png")]
pub use self::png_handler::{format as png_format, format_with as png_format_with};
#[cfg(feature = "tiff")]
pub use self::tiff_handler::{format as tiff_format, format_with as tiff_format_with};
