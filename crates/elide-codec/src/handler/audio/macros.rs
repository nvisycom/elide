//! `impl_audio_handler!`: generate a per-format audio handler + loader +
//! `format()` constructor.
//!
//! WAV and MP3 differ only in their [`FormatId`], lookup keys, and content
//! types; everything else (holding an [`AudioBuffer`], streaming the whole clip
//! as one chunk, resolving a time range to the full clip, and accumulating a
//! redaction batch) is identical and delegates to the standalone
//! [`elide_audio`] engine. The macro stamps out that shared body so the
//! per-format files stay declarative.
//!
//! [`FormatId`]: crate::FormatId
//! [`AudioBuffer`]: elide_audio::AudioBuffer

/// Stamp out the handler, loader, and `format()` for one audio format.
#[cfg(feature = "internal_audio")]
macro_rules! impl_audio_handler {
    (
        handler = $handler:ident,
        loader = $loader:ident,
        format_id = $format_id:literal,
        audio_format = $audio_format:expr,
        extensions = [$($ext:literal),* $(,)?],
        content_types = [$($mime:literal),* $(,)?] $(,)?
    ) => {
        /// Stable [`FormatId`] for this audio codec.
        ///
        /// [`FormatId`]: crate::FormatId
        pub const FORMAT_ID: crate::FormatId = crate::FormatId::new($format_id);

        /// [`Format`] descriptor registered into [`FormatRegistry`].
        ///
        /// [`Format`]: crate::Format
        /// [`FormatRegistry`]: crate::FormatRegistry
        pub fn format() -> crate::Format {
            crate::Format::new::<::elide_audio::modality::Audio, _>(
                FORMAT_ID.clone(),
                $loader,
            )
            .with_extensions([$($ext),*])
            .with_content_types([$($mime),*])
        }

        #[doc = concat!("Handler for a loaded ", $format_id, " clip.")]
        ///
        /// Holds the clip as an [`AudioBuffer`]; redaction accumulates on it and
        /// `encode` applies the batch and re-serializes to the original format.
        /// All sample work delegates to the [`elide_audio`] engine.
        ///
        /// [`AudioBuffer`]: elide_audio::AudioBuffer
        #[derive(Debug)]
        pub(crate) struct $handler {
            clip: ::elide_audio::AudioBuffer,
            yielded: bool,
        }

        impl $handler {
            /// Wrap an opened clip; the streaming cursor starts unyielded.
            pub(crate) fn new(clip: ::elide_audio::AudioBuffer) -> Self {
                Self {
                    clip,
                    yielded: false,
                }
            }
        }

        #[::async_trait::async_trait]
        impl crate::Handler<::elide_audio::modality::Audio> for $handler {
            fn format(&self) -> crate::FormatId {
                FORMAT_ID.clone()
            }

            fn encode(&self) -> ::elide_core::Result<crate::content::ContentData> {
                Ok(crate::content::ContentData::new(self.clip.encode()?))
            }

            async fn read_next(
                &mut self,
            ) -> ::elide_core::Result<
                ::std::option::Option<
                    ::elide_core::modality::Chunk<::elide_audio::modality::Audio>,
                >,
            > {
                if self.yielded {
                    return Ok(None);
                }
                let total_ms = self.clip.duration_ms()?;
                self.yielded = true;
                Ok(Some(::elide_core::modality::Chunk {
                    location: ::elide_audio::modality::AudioLocation::from_millis(0, total_ms),
                    data: ::elide_audio::modality::AudioData::new(self.clip.encode()?),
                    hints: ::std::vec::Vec::new(),
                }))
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_core::modality::DataReader<::elide_audio::modality::Audio> for $handler {
            async fn read_at(
                &self,
                _location: &::elide_audio::modality::AudioLocation,
            ) -> ::elide_core::Result<
                ::std::option::Option<::elide_audio::modality::AudioData>,
            > {
                // The whole clip is the addressable unit; a partial time range
                // still resolves to the full audio for downstream extraction.
                Ok(Some(::elide_audio::modality::AudioData::new(
                    self.clip.encode()?,
                )))
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_core::modality::DataWriter<::elide_audio::modality::Audio> for $handler {
            async fn write_at(
                &mut self,
                redactions: ::elide_core::redaction::Redactions<
                    ::elide_audio::modality::Audio,
                >,
            ) -> ::elide_core::Result<()> {
                self.clip.redact_batch(redactions);
                Ok(())
            }
        }

        /// Loader that opens raw bytes into a
        #[doc = concat!("[`", stringify!($handler), "`].")]
        #[derive(Debug)]
        pub(crate) struct $loader;

        #[::async_trait::async_trait]
        impl crate::Loader<::elide_audio::modality::Audio> for $loader {
            type Handler = $handler;

            async fn decode(
                &self,
                content: crate::content::ContentData,
            ) -> ::elide_core::Result<$handler> {
                let clip = ::elide_audio::AudioBuffer::open(content.as_bytes())?;
                // `AudioBuffer::open` accepts any enabled format; this loader is
                // registered for one, so reject bytes that opened as another
                // (e.g. MP3 content routed to the WAV loader).
                if clip.format() != $audio_format {
                    return ::std::result::Result::Err(::elide_core::Error::new(
                        ::elide_core::ErrorKind::MalformedInput,
                        ::std::format!(
                            "{} loader: content decoded as {:?}, not the expected format",
                            $format_id,
                            clip.format(),
                        ),
                    ));
                }
                Ok($handler::new(clip))
            }
        }
    };
}

#[cfg(feature = "internal_audio")]
pub(crate) use impl_audio_handler;
