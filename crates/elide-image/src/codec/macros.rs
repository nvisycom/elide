//! `impl_image_handler!`: generate a per-format image `format()` constructor.
//!
//! PNG, JPEG and TIFF differ only in their [`FormatId`], lookup keys, and
//! content types; the decode-redact-recompose body is one shared
//! [`ImageDocumentLoader`], which builds a two-part [`Document`] (the pixel
//! [`Stream`] plus the `#exif` [`Blob`]) over this crate's [`ImageBuffer`]
//! engine. The macro stamps out the per-format id and lookup keys so the
//! per-format files stay declarative.
//!
//! [`FormatId`]: elide_codec::FormatId
//! [`Document`]: elide_codec::Document
//! [`Stream`]: elide_codec::Stream
//! [`ImageBuffer`]: crate::ImageBuffer
//! [`ImageDocumentLoader`]: super::document::ImageDocumentLoader

/// Stamp out the `format()` / `format_with()` constructors for one image format.
macro_rules! impl_image_handler {
    (
        format_id = $format_id:literal,
        extensions = [$($ext:literal),* $(,)?],
        content_types = [$($mime:literal),* $(,)?] $(,)?
    ) => {
        /// Stable [`FormatId`] for this image codec.
        ///
        /// [`FormatId`]: elide_codec::FormatId
        pub const FORMAT_ID: ::elide_codec::FormatId = ::elide_codec::FormatId::new($format_id);

        /// [`Format`] descriptor registered into `FormatRegistry`.
        ///
        /// Applies [`ExifPolicy::default`] to the image's EXIF metadata on encode
        /// (strip all but the structurally-required tags), for a build with no
        /// `Metadata` pipeline wired. Use [`format_with`] to set a different
        /// fallback policy.
        ///
        /// [`Format`]: elide_codec::Format
        /// [`ExifPolicy`]: crate::ExifPolicy
        /// [`ExifPolicy::default`]: crate::ExifPolicy
        pub fn format() -> ::elide_codec::Format {
            format_from(::core::default::Default::default())
        }

        /// [`Format`] descriptor with an explicit fallback EXIF policy.
        ///
        /// `policy` governs the metadata of the encoded image **only when no
        /// `Metadata` pipeline touched it** — the redaction path where a wired
        /// `ExifRecognizer` + anonymizer strips fields through the `#exif`
        /// sub-part always wins and ignores `policy`. So this is the "strip all
        /// (or sensitive) EXIF unconditionally, without wiring a metadata
        /// recognizer" knob: pass [`ExifPolicy::Strip`] or
        /// [`StripSensitive`](crate::ExifPolicy::StripSensitive).
        ///
        /// [`Format`]: elide_codec::Format
        /// [`ExifPolicy::Strip`]: crate::ExifPolicy::Strip
        pub fn format_with(policy: crate::ExifPolicy) -> ::elide_codec::Format {
            format_from(policy)
        }

        /// Build this format's [`Format`](elide_codec::Format) from a configured
        /// fallback policy: a [`Document`](elide_codec::Document) format whose
        /// loader is the shared [`ImageDocumentLoader`](super::document::ImageDocumentLoader).
        fn format_from(policy: crate::ExifPolicy) -> ::elide_codec::Format {
            ::elide_codec::Format::with_document_loader(
                FORMAT_ID.clone(),
                super::document::ImageDocumentLoader::new(FORMAT_ID.clone(), policy),
            )
            .with_extensions([$($ext),*])
            .with_content_types([$($mime),*])
        }
    };
}

pub(crate) use impl_image_handler;
