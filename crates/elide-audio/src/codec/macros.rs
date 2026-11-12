//! `impl_audio_handler!`: generate a per-format audio `format()` constructor.
//!
//! WAV and MP3 differ only in their [`FormatId`], lookup keys, and content
//! types; the decode-redact-encode body is one shared [`AudioDocumentLoader`]
//! over this crate's [`AudioBuffer`] engine. The macro stamps out the
//! per-format id and lookup keys so the per-format files stay declarative.
//!
//! [`FormatId`]: elide_codec::FormatId
//! [`AudioBuffer`]: crate::AudioBuffer
//! [`AudioDocumentLoader`]: super::audio_loader::AudioDocumentLoader

/// Stamp out the `format()` constructor for one audio format.
macro_rules! impl_audio_handler {
    (
        format_id = $format_id:literal,
        audio_format = $audio_format:expr,
        extensions = [$($ext:literal),* $(,)?],
        content_types = [$($mime:literal),* $(,)?] $(,)?
    ) => {
        /// Stable [`FormatId`] for this audio codec.
        ///
        /// [`FormatId`]: elide_codec::FormatId
        pub const FORMAT_ID: ::elide_codec::FormatId = ::elide_codec::FormatId::new($format_id);

        /// This container's [`Format`], decoding a clip with its tags stripped.
        ///
        /// Decodes into a one-stream [`Document`](elide_codec::Document) of the
        /// clip.
        ///
        /// [`Format`]: elide_codec::Format
        pub fn format() -> ::elide_codec::Format {
            ::elide_codec::Format::with_document_loader(
                FORMAT_ID.clone(),
                super::audio_loader::AudioDocumentLoader::new(FORMAT_ID.clone(), $audio_format),
            )
            .with_extensions([$($ext),*])
            .with_content_types([$($mime),*])
        }
    };
}

pub(crate) use impl_audio_handler;
