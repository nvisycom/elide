//! `impl_image_handler!`: generate a per-format image handler + loader +
//! `format()` constructor.
//!
//! PNG and JPEG differ only in their [`FormatId`], lookup keys, and
//! content types; everything else (holding a decoded [`ImageBuffer`], the
//! single-chunk streaming, the crop-based read, the redaction pass) is
//! identical and delegates to this crate's [`ImageBuffer`] engine. The macro
//! stamps out that shared body so the per-format files stay declarative.
//!
//! [`FormatId`]: elide_codec::FormatId
//! [`ImageBuffer`]: crate::ImageBuffer

/// Stamp out the handler, loader, and `format()` for one image format.
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
        /// fallback policy.
        fn format_from(policy: crate::ExifPolicy) -> ::elide_codec::Format {
            ::elide_codec::Format::new(FORMAT_ID.clone(), $loader { policy })
            .with_extensions([$($ext),*])
            .with_content_types([$($mime),*])
        }

        #[doc = concat!("Handler for a decoded ", $format_id, " image.")]
        ///
        /// Holds the whole image as an [`ImageBuffer`]; redaction paints over
        /// regions in place and `encode` re-serializes to the original format.
        /// All pixel work delegates to the [`ImageBuffer`] engine.
        ///
        /// [`ImageBuffer`]: crate::ImageBuffer
        #[derive(Debug)]
        pub(crate) struct $handler {
            buffer: crate::ImageBuffer,
            yielded: bool,
            /// The metadata-stripped image bytes folded in from the `#exif`
            /// sub-part, if its metadata was redacted. `encode` lays the pixel
            /// redactions over these; `None` keeps the original metadata.
            metadata: ::std::option::Option<::bytes::Bytes>,
            /// Fallback EXIF policy, applied on encode only when no `Metadata`
            /// pipeline populated `metadata`.
            policy: crate::ExifPolicy,
        }

        impl $handler {
            /// Wrap a decoded image; the streaming cursor starts unyielded.
            pub(crate) fn new(
                buffer: crate::ImageBuffer,
                policy: crate::ExifPolicy,
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
        impl ::elide_codec::Handler<crate::modality::Image> for $handler {
            fn format(&self) -> ::elide_codec::FormatId {
                FORMAT_ID.clone()
            }

            fn encode(&self) -> ::elide_core::Result<::elide_codec::content::ContentData> {
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
                Ok(::elide_codec::content::ContentData::new(bytes))
            }

            fn as_container_mut(&mut self) -> ::std::option::Option<&mut dyn ::elide_codec::Container> {
                ::std::option::Option::Some(self)
            }

            async fn read_next(
                &mut self,
            ) -> ::elide_core::Result<
                ::std::option::Option<::elide_core::modality::Chunk<crate::modality::Image>>,
            > {
                if self.yielded {
                    return Ok(None);
                }
                let dims = self.buffer.dimensions();
                let bbox = crate::primitive::BoundingBox::from_origin(
                    crate::primitive::Point::new(0.0, 0.0),
                    crate::primitive::Dimensions::new(dims.width as f64, dims.height as f64),
                );
                // The detection chunk carries the same fallback policy as the
                // output: a pixel recognizer reads pixels (EXIF detection runs on
                // the `#exif` sub-part, not here), so stripping metadata costs
                // detection nothing and keeps EXIF out of a recognizer call that
                // may leave the process (e.g. a VLM request).
                let data = crate::modality::ImageData::new(
                    self.buffer.encode(self.policy)?,
                );
                self.yielded = true;
                Ok(Some(::elide_core::modality::Chunk {
                    location: crate::modality::ImageLocation::new(bbox),
                    data,
                    hints: ::std::vec::Vec::new(),
                }))
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_core::modality::DataReader<crate::modality::Image> for $handler {
            async fn read_at(
                &self,
                location: &crate::modality::ImageLocation,
            ) -> ::elide_core::Result<
                ::std::option::Option<crate::modality::ImageData>,
            > {
                let dims = self.buffer.dimensions();
                let Some(region) = location.bounding_box.to_pixels(dims) else {
                    return Ok(None);
                };
                self.buffer
                    .crop(region)
                    .map(|raster| raster.encode())
                    .transpose()
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_core::modality::DataWriter<crate::modality::Image> for $handler {
            async fn write_at(
                &mut self,
                redactions: ::elide_core::redaction::Redactions<crate::modality::Image>,
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
        /// Carries the format's fallback [`ExifPolicy`](crate::ExifPolicy),
        /// handed to each decoded handler.
        #[derive(Debug)]
        pub(crate) struct $loader {
            policy: crate::ExifPolicy,
        }

        impl $loader {
            /// A loader with an explicit fallback [`ExifPolicy`], for tests that
            /// exercise a non-default policy directly (the registered path is
            /// [`format_with`]).
            ///
            /// [`ExifPolicy`]: crate::ExifPolicy
            #[allow(dead_code)]
            pub(crate) fn with_policy(policy: crate::ExifPolicy) -> Self {
                Self { policy }
            }
        }

        impl ::std::default::Default for $loader {
            /// The registered default follows [`ExifPolicy::default`]: strip all
            /// but the structurally-required metadata.
            ///
            /// [`ExifPolicy::default`]: crate::ExifPolicy
            fn default() -> Self {
                Self { policy: ::core::default::Default::default() }
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_codec::Loader for $loader {
            type Modality = crate::modality::Image;
            type Handler = $handler;

            async fn decode(
                &self,
                content: ::elide_codec::content::ContentData,
            ) -> ::elide_core::Result<$handler> {
                let buffer =
                    crate::ImageBuffer::open(content.as_bytes())?;
                Ok($handler::new(buffer, self.policy))
            }
        }

        impl ::elide_codec::Container for $handler {
            fn parts(&self) -> ::std::vec::Vec<::elide_codec::Part> {
                // One sub-part: the image's EXIF metadata track, decoded as the
                // metadata modality. Its bytes are the whole image; the metadata
                // handler reads the fields out of them.
                ::std::vec![::elide_codec::Part {
                    id: ::elide_codec::LocalId::new(crate::codec::macros::EXIF_PART_ID),
                    bytes: self.buffer.source_bytes(),
                    hint: crate::codec::macros::EXIF_PART_HINT.to_owned(),
                }]
            }

            fn replace_part(
                &mut self,
                id: &::elide_codec::LocalId,
                bytes: ::bytes::Bytes,
            ) -> ::elide_core::Result<()> {
                if id.as_str() != crate::codec::macros::EXIF_PART_ID {
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
pub(crate) const EXIF_PART_ID: &str = "#exif";

/// The format hint the `#exif` sub-part decodes with — the metadata handler's
/// registered extension, which the fold resolves it by.
pub(crate) const EXIF_PART_HINT: &str = super::exif_handler::EXIF_HINT;

pub(crate) use impl_image_handler;
