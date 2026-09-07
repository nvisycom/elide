//! [`ImageBuffer`]: a decoded raster image, opened once, then read, redacted,
//! and re-encoded, the single entry point into this crate.

mod format;

use bytes::Bytes;
#[cfg(feature = "exif")]
use elide_core::entity::Entity;
use elide_core::modality::image::ImageReplacement;
#[cfg(feature = "exif")]
use elide_core::modality::metadata::{Metadata, MetadataData};
use elide_core::primitive::{Color, Dimensions, PixelRegion};
use elide_core::{Error, ErrorKind, Result};
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

pub use self::format::ImageFormat;
#[cfg(feature = "exif")]
use crate::exif::{ExifPolicy, Source};

/// A decoded raster image, the single entry point into the crate.
///
/// [`open`](Self::open) ingests the bytes once: it detects the format, decodes
/// the pixels, and retains the source container so metadata is available without
/// a second decode or any magic-byte sniffing elsewhere. Reuse the buffer to
/// [`redact`](Self::redact) pixel regions and [`encode`](Self::encode) back out
/// under an [`ExifPolicy`], all paying the decode cost a single time. With the
/// `entities` feature it also surfaces the source's privacy-relevant EXIF fields
/// as [`metadata_entities`](Self::metadata_entities).
#[derive(Debug, Clone)]
pub struct ImageBuffer {
    inner: DynamicImage,
    format: ImageFormat,
    /// The original container bytes, kept so metadata (which the pixel decode
    /// discards) can be read or carried through on encode.
    source: Bytes,
    /// Whether a redaction has changed the pixels since `open`. When clean, an
    /// encode can operate on `source` directly (lossless); when dirty, the
    /// modified pixels must be re-encoded.
    dirty: bool,
}

impl ImageBuffer {
    /// Open `bytes`: detect the format, decode the pixels, and keep the source.
    ///
    /// The one place format is determined; nothing downstream sniffs magic bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the bytes are not a readable image, or
    /// [`ErrorKind::CapabilityUnavailable`] if the format is not one this crate
    /// supports.
    pub fn open(bytes: &[u8]) -> Result<Self> {
        let guessed = image::guess_format(bytes)
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("unknown image: {e}")))?;
        let format = ImageFormat::from_image(guessed)?;
        let inner = image::load_from_memory_with_format(bytes, format.to_image())
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("image decode: {e}")))?;
        Ok(Self {
            inner,
            format,
            source: Bytes::copy_from_slice(bytes),
            dirty: false,
        })
    }

    /// The image's pixel dimensions.
    #[must_use]
    pub fn dimensions(&self) -> Dimensions {
        let (w, h) = self.inner.dimensions();
        Dimensions::new(w, h)
    }

    /// The format the image was opened as, and re-encodes to.
    #[must_use]
    pub fn format(&self) -> ImageFormat {
        self.format
    }

    /// The source container as an EXIF [`Source`], the receiver for metadata ops.
    #[cfg(feature = "exif")]
    fn exif_source(&self) -> Source<'_> {
        Source::new(&self.source, self.format.to_exif())
    }

    /// One entity per privacy-relevant EXIF field the source image carries, so a
    /// metadata field flows through the same detect/select/redact/audit pipeline
    /// as a text span.
    ///
    /// Read from the retained source, so it reflects the original image
    /// regardless of any redaction applied since.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the metadata is present but unparseable.
    #[cfg(feature = "exif")]
    pub fn metadata_entities(&self) -> Result<Vec<Entity<Metadata>>> {
        self.exif_source().entities()
    }

    /// Each privacy-relevant EXIF field as a [`MetadataData`], for a caller that
    /// streams fields to a recognizer (the codec's EXIF handler).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the metadata is present but unparseable.
    #[cfg(feature = "exif")]
    pub fn metadata_fields(&self) -> Result<Vec<MetadataData>> {
        self.exif_source().fields()
    }

    /// The original source container bytes, for a caller that needs to decode
    /// the image's metadata track separately (the codec's `#exif` sub-part).
    #[cfg(feature = "exif")]
    #[must_use]
    pub fn source_bytes(&self) -> Bytes {
        self.source.clone()
    }

    /// Remove the EXIF fields named by `keys` from the source container and run
    /// the format's baseline scrub, returning the metadata-edited bytes. The
    /// pixels are untouched; this is the source's metadata track only.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if a metadata edit fails.
    #[cfg(feature = "exif")]
    pub fn strip_metadata_keys(&self, keys: &[&str]) -> Result<Bytes> {
        Ok(self.exif_source().remove_keys(keys)?.into())
    }

    /// Re-encode the image under `policy`, returning the container bytes.
    ///
    /// When no redaction has been applied, an EXIF strip edits the source
    /// container losslessly (no pixel recompression) and [`Keep`](ExifPolicy::Keep)
    /// returns the source untouched. When pixels have been redacted, the modified
    /// image is re-encoded, and [`Keep`](ExifPolicy::Keep) carries the source's
    /// metadata onto the result (the decode dropped it).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if the encoder rejects the image, or
    /// [`ErrorKind::MalformedInput`] if a metadata edit fails.
    #[cfg(feature = "exif")]
    pub fn encode(&self, policy: ExifPolicy) -> Result<Bytes> {
        if !self.dirty {
            // Pixels unchanged: act on the source container directly (lossless).
            return Ok(self.exif_source().strip(policy)?.into());
        }
        // Pixels redacted: re-encode them. The re-encoded bytes carry no EXIF,
        // so a strip is already satisfied; Keep must carry the source's over.
        let encoded = Self::encode_image(&self.inner, self.format)?;
        Ok(match policy {
            ExifPolicy::StripAll | ExifPolicy::StripSensitive => encoded,
            ExifPolicy::Keep => self.exif_source().transfer(encoded.into())?.into(),
        })
    }

    /// Re-encode the image, returning the container bytes.
    ///
    /// Without the `exif` feature the crate does not touch metadata: an
    /// unredacted image round-trips its source bytes; a redacted one re-encodes
    /// its pixels (any source metadata the decode dropped is not carried over).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if the encoder rejects the image.
    #[cfg(not(feature = "exif"))]
    pub fn encode(&self) -> Result<Bytes> {
        if !self.dirty {
            return Ok(self.source.clone());
        }
        Self::encode_image(&self.inner, self.format)
    }

    /// Re-encode the current (possibly redacted) pixels, carrying the EXIF
    /// metadata from `container` rather than from this image's own source.
    ///
    /// The composition step for an image whose metadata was edited separately:
    /// `container` is the metadata-stripped image the `#exif` sub-part produced,
    /// and this lays the redacted pixels over it, so the one output has both the
    /// pixel redactions and exactly the metadata the sub-part kept. When the
    /// pixels are unchanged, `container` already *is* the answer (its pixels are
    /// the originals), so it is returned as-is.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if the encoder rejects the image, or
    /// [`ErrorKind::MalformedInput`] if the metadata transfer fails.
    #[cfg(feature = "exif")]
    pub fn encode_over_metadata(&self, container: &[u8]) -> Result<Bytes> {
        if !self.dirty {
            // Pixels untouched: the sub-part's container already carries the
            // original pixels plus its metadata edit.
            return Ok(Bytes::copy_from_slice(container));
        }
        let encoded = Self::encode_image(&self.inner, self.format)?;
        let source = Source::new(container, self.format.to_exif());
        Ok(source.transfer(encoded.into())?.into())
    }

    /// Crop `region` out and encode it, or `None` when the region is empty (out
    /// of bounds or zero-area). The crop carries no metadata.
    pub fn crop(&self, region: PixelRegion) -> Result<Option<Bytes>> {
        if region.width == 0 || region.height == 0 {
            return Ok(None);
        }
        let cropped = self
            .inner
            .crop_imm(region.x, region.y, region.width, region.height);
        Ok(Some(Self::encode_image(&cropped, self.format)?))
    }

    /// Paint `replacement` over `region` in place (blur, pixelate, block, or
    /// remove). A zero-area region is a silent no-op; redaction is best-effort
    /// over whatever pixels exist.
    pub fn redact(&mut self, region: PixelRegion, replacement: &ImageReplacement) {
        match replacement {
            ImageReplacement::Blur { sigma } => self.blur(region, *sigma),
            ImageReplacement::Pixelate { block_size } => self.pixelate(region, *block_size),
            ImageReplacement::Block { color } => self.block(region, *color),
            ImageReplacement::Removed => self.block(region, Color::BLACK),
            ImageReplacement::Unchanged => return,
        }
        self.dirty = true;
    }

    /// Gaussian blur over the region: crop, blur the crop, overlay it back.
    fn blur(&mut self, region: PixelRegion, sigma: f32) {
        let sub = self
            .inner
            .crop_imm(region.x, region.y, region.width, region.height)
            .to_rgba8();
        let blurred = imageproc::filter::gaussian_blur_f32(&sub, sigma.max(f32::MIN_POSITIVE));
        self.overlay(&DynamicImage::ImageRgba8(blurred), region);
    }

    /// Solid-color block over the region.
    fn block(&mut self, region: PixelRegion, color: Color) {
        let fill = RgbaImage::from_pixel(
            region.width,
            region.height,
            Rgba([color.r, color.g, color.b, 255]),
        );
        self.overlay(&DynamicImage::ImageRgba8(fill), region);
    }

    /// Mosaic pixelation: downscale the region with nearest-neighbor, scale it
    /// back up, and overlay.
    fn pixelate(&mut self, region: PixelRegion, block_size: u32) {
        let block_size = block_size.max(1);
        let small_w = (region.width / block_size).max(1);
        let small_h = (region.height / block_size).max(1);
        let sub = self
            .inner
            .crop_imm(region.x, region.y, region.width, region.height);
        let small = sub.resize_exact(small_w, small_h, FilterType::Nearest);
        let mosaic = small.resize_exact(region.width, region.height, FilterType::Nearest);
        self.overlay(&mosaic, region);
    }

    /// Overlay `patch` onto the image at the region's top-left corner.
    fn overlay(&mut self, patch: &DynamicImage, region: PixelRegion) {
        image::imageops::overlay(&mut self.inner, patch, region.x as i64, region.y as i64);
    }

    /// Encode `img` to bytes in `format`.
    fn encode_image(img: &DynamicImage, format: ImageFormat) -> Result<Bytes> {
        use std::io::Cursor;

        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, format.to_image())
            .map_err(|e| Error::new(ErrorKind::Processing, format!("image encode: {e}")))?;
        Ok(Bytes::from(buf.into_inner()))
    }
}
