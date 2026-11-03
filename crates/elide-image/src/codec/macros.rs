//! `impl_image_handler!`: generate a per-format image `format()` constructor.
//!
//! PNG, JPEG and TIFF differ only in their [`FormatId`], lookup keys, and
//! content types; the decode-redact-recompose body is one shared
//! [`ImageDocumentLoader`], which builds a [`Document`] (the pixel [`Stream`],
//! plus the `#exif` metadata stream when metadata is treated as content) over
//! this crate's [`ImageBuffer`] engine. The macro stamps out the per-format id
//! and lookup keys so the per-format files stay declarative.
//!
//! [`FormatId`]: elide_codec::FormatId
//! [`Document`]: elide_codec::Document
//! [`Stream`]: elide_codec::Stream
//! [`ImageBuffer`]: crate::ImageBuffer
//! [`ImageDocumentLoader`]: super::image_loader::ImageDocumentLoader

/// Stamp out the `format()` constructor for one image format.
macro_rules! impl_image_handler {
    (
        format_id = $format_id:literal,
        image_format = $image_format:expr,
        extensions = [$($ext:literal),* $(,)?],
        content_types = [$($mime:literal),* $(,)?] $(,)?
    ) => {
        /// Stable [`FormatId`] for this image codec.
        ///
        /// [`FormatId`]: elide_codec::FormatId
        pub const FORMAT_ID: ::elide_codec::FormatId = ::elide_codec::FormatId::new($format_id);

        /// [`Format`] descriptor registered into `FormatRegistry`. Decodes into a
        /// [`Document`](elide_codec::Document) of the pixel stream, plus the
        /// `#exif` metadata stream when the hidden-content policy treats metadata
        /// as content.
        ///
        /// [`Format`]: elide_codec::Format
        pub fn format() -> ::elide_codec::Format {
            ::elide_codec::Format::with_document_loader(
                FORMAT_ID.clone(),
                super::image_loader::ImageDocumentLoader::new(FORMAT_ID.clone(), $image_format),
            )
            .with_extensions([$($ext),*])
            .with_content_types([$($mime),*])
        }
    };
}

pub(crate) use impl_image_handler;
