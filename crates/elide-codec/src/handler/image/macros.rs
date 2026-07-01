//! `impl_image_handler!`: generate a per-format image handler + loader +
//! `format()` constructor.
//!
//! PNG, JPEG, and TIFF differ only in their [`FormatId`], lookup keys,
//! and the [`image::ImageFormat`] used to re-encode; everything else (the
//! decoded [`DynamicImage`] they hold, the single-chunk streaming, the
//! crop-based read, the redaction pass) is identical. The macro stamps
//! out that shared body so the per-format files stay declarative.
//!
//! [`FormatId`]: crate::FormatId

/// Encode a [`DynamicImage`] to bytes in `fmt`.
///
/// Shared by every generated handler's `encode`/`read_next`/`read_at`.
///
/// [`DynamicImage`]: image::DynamicImage
pub(crate) fn encode_image(
    img: &image::DynamicImage,
    fmt: image::ImageFormat,
) -> elide_core::Result<bytes::Bytes> {
    use std::io::Cursor;

    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, fmt).map_err(|e| {
        elide_core::Error::new(
            elide_core::ErrorKind::Validation,
            format!("image encode failed: {e}"),
        )
    })?;
    Ok(bytes::Bytes::from(buf.into_inner()))
}

/// Stamp out the handler, loader, and `format()` for one image format.
///
/// Only defined when at least one image format is enabled; `internal_image`
/// can also be pulled on its own (e.g. by `pdf-render`, which reuses
/// [`encode_image`] without any format handler).
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
macro_rules! impl_image_handler {
    (
        handler = $handler:ident,
        loader = $loader:ident,
        format_id = $format_id:literal,
        extensions = [$($ext:literal),* $(,)?],
        content_types = [$($mime:literal),* $(,)?],
        image_format = $img_fmt:expr $(,)?
    ) => {
        /// Stable [`FormatId`] for this image codec.
        ///
        /// [`FormatId`]: crate::FormatId
        pub const FORMAT_ID: crate::FormatId = crate::FormatId::new($format_id);

        /// [`Format`] descriptor registered into [`FormatRegistry`].
        ///
        /// [`Format`]: crate::Format
        /// [`FormatRegistry`]: crate::FormatRegistry
        pub fn format() -> crate::Format {
            crate::Format::new::<::elide_core::modality::image::Image, _>(FORMAT_ID.clone(), $loader)
                .with_extensions([$($ext),*])
                .with_content_types([$($mime),*])
        }

        #[doc = concat!("Handler for a decoded ", $format_id, " image.")]
        ///
        /// Holds the whole image in memory as a
        /// [`DynamicImage`]; redaction paints over
        /// regions in place and `encode` re-serializes to the original
        /// format.
        ///
        /// [`DynamicImage`]: image::DynamicImage
        #[derive(Debug)]
        pub(crate) struct $handler {
            image: ::image::DynamicImage,
            yielded: bool,
        }

        impl $handler {
            /// Wrap a decoded image; the streaming cursor starts unyielded.
            pub(crate) fn new(image: ::image::DynamicImage) -> Self {
                Self { image, yielded: false }
            }
        }

        #[::async_trait::async_trait]
        impl crate::Handler<::elide_core::modality::image::Image> for $handler {
            fn format(&self) -> crate::FormatId {
                FORMAT_ID.clone()
            }

            fn encode(&self) -> ::elide_core::Result<crate::content::ContentData> {
                let bytes = $crate::handler::image::macros::encode_image(&self.image, $img_fmt)?;
                Ok(crate::content::ContentData::new(bytes))
            }

            async fn read_next(
                &mut self,
            ) -> ::elide_core::Result<
                ::std::option::Option<::elide_core::modality::Chunk<::elide_core::modality::image::Image>>,
            > {
                use ::image::GenericImageView;

                if self.yielded {
                    return Ok(None);
                }
                let (w, h) = self.image.dimensions();
                let bbox = ::elide_core::primitive::BoundingBox::from_origin_size(
                    ::elide_core::primitive::Point::new(0.0, 0.0),
                    w as f64,
                    h as f64,
                );
                let bytes = $crate::handler::image::macros::encode_image(&self.image, $img_fmt)?;
                let data = ::elide_core::modality::image::ImageData::new(
                    bytes,
                    ::elide_core::primitive::Dimensions::new(w, h),
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
                use ::image::GenericImageView;

                let (img_w, img_h) = self.image.dimensions();
                let dims = ::elide_core::primitive::Dimensions::new(img_w, img_h);
                let Some(region) = location.bounding_box.to_pixels(dims) else {
                    return Ok(None);
                };
                let cropped =
                    self.image
                        .crop_imm(region.x, region.y, region.width, region.height);
                let bytes = $crate::handler::image::macros::encode_image(&cropped, $img_fmt)?;
                Ok(Some(::elide_core::modality::image::ImageData::new(
                    bytes,
                    region.dimensions(),
                )))
            }
        }

        #[::async_trait::async_trait]
        impl ::elide_core::modality::DataWriter<::elide_core::modality::image::Image> for $handler {
            async fn write_at(
                &mut self,
                redactions: ::elide_core::operator::Redactions<::elide_core::modality::image::Image>,
            ) -> ::elide_core::Result<()> {
                for (location, replacement) in redactions.into_iter() {
                    $crate::handler::image::redact::apply(
                        &mut self.image,
                        &replacement,
                        &location.bounding_box,
                    );
                }
                Ok(())
            }
        }

        /// Loader that decodes raw bytes into a
        #[doc = concat!("[`", stringify!($handler), "`].")]
        #[derive(Debug)]
        pub(crate) struct $loader;

        #[::async_trait::async_trait]
        impl crate::Loader<::elide_core::modality::image::Image> for $loader {
            type Handler = $handler;

            async fn decode(
                &self,
                content: crate::content::ContentData,
            ) -> ::elide_core::Result<$handler> {
                let image = ::image::load_from_memory(content.as_bytes()).map_err(|e| {
                    ::elide_core::Error::new(
                        ::elide_core::ErrorKind::Validation,
                        format!(concat!($format_id, " decode failed: {}"), e),
                    )
                })?;
                Ok($handler::new(image))
            }
        }
    };
}

#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
pub(crate) use impl_image_handler;
