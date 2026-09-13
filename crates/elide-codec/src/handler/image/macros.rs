//! `impl_image_handler!`: generate a per-format image handler + loader +
//! `format()` constructor.
//!
//! PNG and JPEG differ only in their [`FormatId`], lookup keys, and
//! content types; everything else (holding a
//! decoded [`ImageBuffer`], the single-chunk streaming, the crop-based read, the
//! redaction pass) is identical and delegates to the standalone
//! [`elide_image`] engine. The macro stamps out that shared body so the
//! per-format files stay declarative.
//!
//! [`FormatId`]: crate::FormatId
//! [`ImageBuffer`]: elide_image::ImageBuffer

/// Stamp out the handler, loader, and `format()` for one image format.
///
/// Only defined when at least one image format is enabled; `internal_image` can
/// also be pulled on its own (e.g. by `pdf-render`, which decodes and redacts a
/// PDF's embedded images without instantiating a format handler).
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
macro_rules! impl_image_handler {
    (
        handler = $handler:ident,
        loader = $loader:ident,
        format_id = $format_id:literal,
        extensions = [$($ext:literal),* $(,)?],
        content_types = [$($mime:literal),* $(,)?] $(,)?
    ) => {
        /// Stable [`FormatId`] for this image codec.
        ///
        /// [`FormatId`]: crate::FormatId
        pub const FORMAT_ID: crate::FormatId = crate::FormatId::new($format_id);

        /// [`Format`] descriptor registered into [`FormatRegistry`].
        ///
        /// Keeps the image's EXIF metadata on encode. Use [`format_with`] to set
        /// a different fallback [`ExifPolicy`] for a build with no `Metadata`
        /// pipeline wired.
        ///
        /// [`Format`]: crate::Format
        /// [`FormatRegistry`]: crate::FormatRegistry
        /// [`ExifPolicy`]: elide_image::ExifPolicy
        pub fn format() -> crate::Format {
            format_from(::elide_image::ExifPolicy::Keep)
        }

        /// [`Format`] descriptor with an explicit fallback EXIF policy.
        ///
        /// `policy` governs the metadata of the encoded image **only when no
        /// `Metadata` pipeline touched it** — the redaction path where a wired
        /// `ExifRecognizer` + anonymizer strips fields through the `#exif`
        /// sub-part always wins and ignores `policy`. So this is the "strip all
        /// (or sensitive) EXIF unconditionally, without wiring a metadata
        /// recognizer" knob: pass [`ExifPolicy::StripAll`] or
        /// [`StripSensitive`](elide_image::ExifPolicy::StripSensitive).
        ///
        /// [`Format`]: crate::Format
        /// [`ExifPolicy::StripAll`]: elide_image::ExifPolicy::StripAll
        pub fn format_with(policy: ::elide_image::ExifPolicy) -> crate::Format {
            format_from(policy)
        }

        /// Build this format's [`Format`](crate::Format) from a configured
        /// fallback policy.
        fn format_from(policy: ::elide_image::ExifPolicy) -> crate::Format {
            crate::Format::new::<::elide_core::modality::image::Image, _>(
                FORMAT_ID.clone(),
                $loader { policy },
            )
            .with_extensions([$($ext),*])
            .with_content_types([$($mime),*])
        }

        #[doc = concat!("Handler for a decoded ", $format_id, " image.")]
        ///
        /// Holds the whole image as an [`ImageBuffer`]; redaction paints over
        /// regions in place and `encode` re-serializes to the original format.
        /// All pixel work delegates to the [`elide_image`] engine.
        ///
        /// [`ImageBuffer`]: elide_image::ImageBuffer
        #[derive(Debug)]
        pub(crate) struct $handler {
            buffer: ::elide_image::ImageBuffer,
            yielded: bool,
            /// The metadata-stripped image bytes folded in from the `#exif`
            /// sub-part, if its metadata was redacted. `encode` lays the pixel
            /// redactions over these; `None` keeps the original metadata.
            metadata: ::std::option::Option<::bytes::Bytes>,
            /// Fallback EXIF policy, applied on encode only when no `Metadata`
            /// pipeline populated `metadata`.
            policy: ::elide_image::ExifPolicy,
        }

        impl $handler {
            /// Wrap a decoded image; the streaming cursor starts unyielded.
            pub(crate) fn new(
                buffer: ::elide_image::ImageBuffer,
                policy: ::elide_image::ExifPolicy,
            ) -> Self {
                Self {
                    buffer,
                    yielded: false,
                    metadata: ::std::option::Option::None,
                    policy,
                }
            }
        }

        #[::async_trait::async_trait]
        impl crate::Handler<::elide_core::modality::image::Image> for $handler {
            fn format(&self) -> crate::FormatId {
                FORMAT_ID.clone()
            }

            fn encode(&self) -> ::elide_core::Result<crate::content::ContentData> {
                // Compose the two tracks into one image: if the `#exif` sub-part
                // edited the metadata, lay the pixel redactions over its stripped
                // container; otherwise re-encode pixels keeping the metadata.
                let bytes = match &self.metadata {
                    ::std::option::Option::Some(container) =>
                        self.buffer.encode_over_metadata(container)?,
                    // No `Metadata` pipeline ran; apply the format's fallback
                    // EXIF policy.
                    ::std::option::Option::None =>
                        self.buffer.encode(self.policy)?,
                };
                Ok(crate::content::ContentData::new(bytes))
            }

            fn as_container_mut(&mut self) -> ::std::option::Option<&mut dyn crate::Container> {
                ::std::option::Option::Some(self)
            }

            async fn read_next(
                &mut self,
            ) -> ::elide_core::Result<
                ::std::option::Option<::elide_core::modality::Chunk<::elide_core::modality::image::Image>>,
            > {
                if self.yielded {
                    return Ok(None);
                }
                let dims = self.buffer.dimensions();
                let bbox = ::elide_core::primitive::BoundingBox::from_origin_size(
                    ::elide_core::primitive::Point::new(0.0, 0.0),
                    dims.width as f64,
                    dims.height as f64,
                );
                let data = ::elide_core::modality::image::ImageData::new(
                    self.buffer.encode(::elide_image::ExifPolicy::Keep)?,
                    dims,
                );
                self.yielded = true;
                Ok(Some(::elide_core::modality::Chunk {
                    location: ::elide_core::modality::image::ImageLocation::new(bbox),
                    data,
                    hints: ::std::vec::Vec::new(),
                }))
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_core::modality::DataReader<::elide_core::modality::image::Image> for $handler {
            async fn read_at(
                &self,
                location: &::elide_core::modality::image::ImageLocation,
            ) -> ::elide_core::Result<
                ::std::option::Option<::elide_core::modality::image::ImageData>,
            > {
                let dims = self.buffer.dimensions();
                let Some(region) = location.bounding_box.to_pixels(dims) else {
                    return Ok(None);
                };
                let region_dims = region.dimensions();
                Ok(self.buffer.crop(region)?.map(|bytes| {
                    ::elide_core::modality::image::ImageData::new(bytes, region_dims)
                }))
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_core::modality::DataWriter<::elide_core::modality::image::Image> for $handler {
            async fn write_at(
                &mut self,
                redactions: ::elide_core::redaction::Redactions<::elide_core::modality::image::Image>,
            ) -> ::elide_core::Result<()> {
                let dims = self.buffer.dimensions();
                for (location, replacement) in redactions.into_iter() {
                    if let Some(region) = location.bounding_box.to_pixels(dims) {
                        self.buffer.redact(region, &replacement);
                    }
                }
                Ok(())
            }
        }

        /// Loader that decodes raw bytes into a
        #[doc = concat!("[`", stringify!($handler), "`].")]
        ///
        /// Carries the format's fallback [`ExifPolicy`](elide_image::ExifPolicy),
        /// handed to each decoded handler.
        #[derive(Debug)]
        pub(crate) struct $loader {
            policy: ::elide_image::ExifPolicy,
        }

        impl $loader {
            /// A loader with an explicit fallback [`ExifPolicy`], for tests that
            /// exercise a non-default policy directly (the registered path is
            /// [`format_with`]).
            ///
            /// [`ExifPolicy`]: elide_image::ExifPolicy
            #[allow(dead_code)]
            pub(crate) fn with_policy(policy: ::elide_image::ExifPolicy) -> Self {
                Self { policy }
            }
        }

        impl ::std::default::Default for $loader {
            /// The registered default: keep the image's EXIF (the `format()`
            /// policy), not `ExifPolicy`'s own `StripAll` default.
            fn default() -> Self {
                Self { policy: ::elide_image::ExifPolicy::Keep }
            }
        }

        #[::async_trait::async_trait]
        impl crate::Loader<::elide_core::modality::image::Image> for $loader {
            type Handler = $handler;

            async fn decode(
                &self,
                content: crate::content::ContentData,
            ) -> ::elide_core::Result<$handler> {
                let buffer =
                    ::elide_image::ImageBuffer::open(content.as_bytes())?;
                Ok($handler::new(buffer, self.policy))
            }
        }

        impl crate::Container for $handler {
            fn parts(&self) -> ::std::vec::Vec<crate::Part> {
                // One sub-part: the image's EXIF metadata track, decoded as the
                // metadata modality. Its bytes are the whole image; the metadata
                // handler reads the fields out of them.
                ::std::vec![crate::Part {
                    id: crate::LocalId::new(crate::handler::image::macros::EXIF_PART_ID),
                    bytes: self.buffer.source_bytes(),
                    hint: crate::handler::image::macros::EXIF_PART_HINT.to_owned(),
                }]
            }

            fn replace_part(
                &mut self,
                id: &crate::LocalId,
                bytes: ::bytes::Bytes,
            ) -> ::elide_core::Result<()> {
                if id.as_str() != crate::handler::image::macros::EXIF_PART_ID {
                    return ::std::result::Result::Err(::elide_core::Error::new(
                        ::elide_core::ErrorKind::MalformedInput,
                        ::std::format!("image replace_part: `{id}` is not the `#exif` sub-part"),
                    ));
                }
                // The metadata-stripped image the `#exif` handler produced;
                // `encode` lays the pixel redactions over it.
                self.metadata = ::std::option::Option::Some(bytes);
                ::std::result::Result::Ok(())
            }
        }
    };
}

/// The local id of the image's EXIF metadata sub-part.
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
pub(crate) const EXIF_PART_ID: &str = "#exif";

/// The format hint the `#exif` sub-part decodes with — the metadata handler's
/// registered extension, which the fold resolves it by.
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
pub(crate) const EXIF_PART_HINT: &str = super::exif_handler::EXIF_HINT;

#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
pub(crate) use impl_image_handler;
