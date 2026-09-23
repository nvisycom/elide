//! [`RasterImage`]: a decoded image and its format, the pixel-editing core.

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

use crate::modality::{ImageData, ImageFormat, ImageReplacement};
use crate::primitive::{BoundingBox, Color, Dimensions, Point};

/// A decoded image plus the format it encodes to: the pixel-level working type.
///
/// Where [`ImageBuffer`](super::ImageBuffer) is an *opened source document*
/// (this image plus its original container and metadata), a `RasterImage` is
/// just the pixels — no source bytes, no metadata. Every pixel operation lives
/// here: [`crop`], [`redact`], and encoding to an [`ImageData`]. A crop is
/// itself a `RasterImage` (a smaller image with no source of its own), so it
/// encodes through the same [`encode`] path rather than a special case.
///
/// [`crop`]: Self::crop
/// [`redact`]: Self::redact
/// [`encode`]: Self::encode
#[derive(Debug, Clone)]
pub struct RasterImage {
    inner: DynamicImage,
    format: ImageFormat,
}

impl RasterImage {
    /// Wrap a decoded image that encodes to `format`.
    pub(super) fn new(inner: DynamicImage, format: ImageFormat) -> Self {
        Self { inner, format }
    }

    /// The image's pixel dimensions.
    #[must_use]
    pub fn dimensions(&self) -> Dimensions<u32> {
        let (w, h) = self.inner.dimensions();
        Dimensions::new(w, h)
    }

    /// The format the image encodes to.
    #[must_use]
    pub fn format(&self) -> ImageFormat {
        self.format
    }

    /// Encode the current pixels to an [`ImageData`] in this image's format.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if the encoder rejects the image.
    pub fn encode(&self) -> Result<ImageData> {
        self.encode_as(self.format)
    }

    /// Encode the current pixels to an [`ImageData`] in `format`, a transcode for
    /// a caller that needs a specific container regardless of the source format
    /// (a vision model that accepts only PNG/JPEG, given a TIFF).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if the encoder rejects the image.
    pub fn encode_as(&self, format: ImageFormat) -> Result<ImageData> {
        Ok(ImageData::new(self.encode_bytes(format)?))
    }

    /// The current pixels as raw container bytes in this image's format, for the
    /// [`ImageBuffer`](super::ImageBuffer) metadata path that lays EXIF over the
    /// re-encoded image.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if the encoder rejects the image.
    pub(super) fn encode_raw(&self) -> Result<Bytes> {
        self.encode_bytes(self.format)
    }

    /// The current pixels as raw container bytes in `format`.
    fn encode_bytes(&self, format: ImageFormat) -> Result<Bytes> {
        use std::io::Cursor;

        let mut buf = Cursor::new(Vec::new());
        self.inner
            .write_to(&mut buf, format.to_image())
            .map_err(|e| Error::new(ErrorKind::Processing, format!("image encode: {e}")))?;
        Ok(Bytes::from(buf.into_inner()))
    }

    /// Crop the in-bounds intersection of `region` out as its own image, or
    /// `None` when the region does not overlap the image (out of bounds or
    /// zero-area). The crop carries no metadata.
    #[must_use]
    pub fn crop(&self, region: BoundingBox<u32>) -> Option<Self> {
        let region = self.clamp(region)?;
        let cropped =
            self.inner
                .crop_imm(region.left(), region.top(), region.width(), region.height());
        Some(Self {
            inner: cropped,
            format: self.format,
        })
    }

    /// Paint `replacement` over `region` in place (blur, pixelate, block, or
    /// remove), returning whether any pixel changed. A region that does not
    /// overlap the image is a silent no-op that reports `false`; redaction acts
    /// on the in-bounds intersection.
    pub fn redact(&mut self, region: BoundingBox<u32>, replacement: &ImageReplacement) -> bool {
        let Some(region) = self.clamp(region) else {
            return false;
        };
        match replacement {
            ImageReplacement::Blur { sigma } => self.blur(region, *sigma),
            ImageReplacement::Pixelate { block_size } => self.pixelate(region, *block_size),
            ImageReplacement::Block { color } => self.block(region, *color),
            ImageReplacement::Removed => self.block(region, Color::BLACK),
            ImageReplacement::Unchanged => return false,
        }
        true
    }

    /// `region` clamped to the image bounds, or `None` when the overlap is empty.
    ///
    /// A caller can hand in a region that runs past the edges or lies wholly
    /// outside. `image`'s `crop_imm` silently clips such a region, so a
    /// wholly-outside region would collapse to a zero-sized crop, and a partly
    /// outside one would act on fewer pixels than the caller named. Resolving the
    /// intersection here makes both cases explicit: a real overlap is clamped to
    /// exactly the in-bounds pixels, and no overlap is `None`.
    fn clamp(&self, region: BoundingBox<u32>) -> Option<BoundingBox<u32>> {
        let (w, h) = self.inner.dimensions();
        let x = region.left().min(w);
        let y = region.top().min(h);
        // Saturating: a caller-supplied region near `u32::MAX` must not overflow
        // the edge sum (`x + width`), which would panic in debug and wrap in
        // release. The edges are clamped to the image, so saturation is harmless.
        let right = region.left().saturating_add(region.width()).min(w);
        let bottom = region.top().saturating_add(region.height()).min(h);
        if right <= x || bottom <= y {
            return None;
        }
        Some(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(right - x, bottom - y),
        ))
    }

    /// Gaussian blur over the region: crop, blur the crop, overlay it back.
    fn blur(&mut self, region: BoundingBox<u32>, sigma: f32) {
        let sub = self
            .inner
            .crop_imm(region.left(), region.top(), region.width(), region.height())
            .to_rgba8();
        let blurred = imageproc::filter::gaussian_blur_f32(&sub, sigma.max(f32::MIN_POSITIVE));
        self.overlay(&DynamicImage::ImageRgba8(blurred), region);
    }

    /// Solid-color block over the region.
    fn block(&mut self, region: BoundingBox<u32>, color: Color) {
        let fill = RgbaImage::from_pixel(
            region.width(),
            region.height(),
            Rgba([color.r, color.g, color.b, 255]),
        );
        self.overlay(&DynamicImage::ImageRgba8(fill), region);
    }

    /// Mosaic pixelation: downscale the region with nearest-neighbor, scale it
    /// back up, and overlay.
    fn pixelate(&mut self, region: BoundingBox<u32>, block_size: u32) {
        let block_size = block_size.max(1);
        let small_w = (region.width() / block_size).max(1);
        let small_h = (region.height() / block_size).max(1);
        let sub = self
            .inner
            .crop_imm(region.left(), region.top(), region.width(), region.height());
        let small = sub.resize_exact(small_w, small_h, FilterType::Nearest);
        let mosaic = small.resize_exact(region.width(), region.height(), FilterType::Nearest);
        self.overlay(&mosaic, region);
    }

    /// Overlay `patch` onto the image at the region's top-left corner.
    fn overlay(&mut self, patch: &DynamicImage, region: BoundingBox<u32>) {
        image::imageops::overlay(
            &mut self.inner,
            patch,
            region.left() as i64,
            region.top() as i64,
        );
    }
}
