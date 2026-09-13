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
use crate::exif::Source;
#[cfg(feature = "exif")]
use crate::policy::ExifPolicy;

/// A decoded raster image, the single entry point into the crate.
///
/// [`open`](Self::open) ingests the bytes once: it detects the format, decodes
/// the pixels, and retains the source container so metadata is available without
/// a second decode or any magic-byte sniffing elsewhere. Reuse the buffer to
/// [`redact`](Self::redact) pixel regions and [`encode`](Self::encode) back out
/// under an [`ExifPolicy`](crate::ExifPolicy), all paying the decode cost a
/// single time. With the `exif` feature it also surfaces the source's
/// privacy-relevant EXIF fields as `Entity<Metadata>` values.
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
    /// # Coordinate space
    ///
    /// The pixels are decoded as stored, with the EXIF `Orientation` tag *not*
    /// applied, so a coordinate is a raw stored-pixel coordinate, not a
    /// display-space one. This is deliberate: detection reads these same stored
    /// pixels, so a region it reports and a region [`redact`](Self::redact) paints
    /// share one coordinate system and always line up. A caller that holds
    /// display-space coordinates (e.g. from a viewer that honours `Orientation`)
    /// must map them into stored space before calling in, or the wrong pixels are
    /// edited. [`dimensions`](Self::dimensions) likewise reports stored size.
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
    /// image is re-encoded (the fresh bytes carry no EXIF), and the source's
    /// metadata is carried onto the result according to `policy`:
    /// [`Keep`](ExifPolicy::Keep) transfers all of it,
    /// [`StripSensitive`](ExifPolicy::StripSensitive) transfers only the
    /// non-sensitive fields the policy retains (e.g. `Orientation`), and
    /// [`StripAll`](ExifPolicy::StripAll) transfers none.
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
        // Pixels redacted: re-encode them. The fresh bytes carry no EXIF, so the
        // metadata to keep is transferred from the source onto them.
        let encoded = Self::encode_image(&self.inner, self.format)?;
        Ok(match policy {
            // Nothing to carry: the re-encoded bytes already hold no metadata.
            ExifPolicy::StripAll => encoded,
            // Transfer everything from the original source.
            ExifPolicy::Keep => self.exif_source().transfer(encoded.into())?.into(),
            // Transfer only what survives a sensitive strip: apply the strip to
            // the source container first, then carry its remaining (non-sensitive)
            // metadata onto the fresh pixels, so benign fields like Orientation
            // are retained rather than dropped with everything else.
            ExifPolicy::StripSensitive => {
                let retained = self.exif_source().strip(ExifPolicy::StripSensitive)?;
                Source::new(&retained, self.format.to_exif())
                    .transfer(encoded.into())?
                    .into()
            }
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

    /// `region` clamped to the image bounds, or `None` when the overlap is empty.
    ///
    /// A caller can hand in a region that runs past the edges or lies wholly
    /// outside. `image`'s `crop_imm` silently clips such a region, so a
    /// wholly-outside region would collapse to a zero-sized crop, and a partly
    /// outside one would act on fewer pixels than the caller named. Resolving the
    /// intersection here makes both cases explicit: a real overlap is clamped to
    /// exactly the in-bounds pixels, and no overlap is `None`.
    fn clamp(&self, region: PixelRegion) -> Option<PixelRegion> {
        let (w, h) = self.inner.dimensions();
        let x = region.x.min(w);
        let y = region.y.min(h);
        // Saturating: a caller-supplied region near `u32::MAX` must not overflow
        // the edge sum (`x + width`), which would panic in debug and wrap in
        // release. The edges are clamped to the image, so saturation is harmless.
        let right = region.x.saturating_add(region.width).min(w);
        let bottom = region.y.saturating_add(region.height).min(h);
        if right <= x || bottom <= y {
            return None;
        }
        Some(PixelRegion::new(x, y, right - x, bottom - y))
    }

    /// Crop `region` out and encode it, or `None` when the region does not
    /// overlap the image (out of bounds or zero-area). The crop is the in-bounds
    /// intersection and carries no metadata.
    pub fn crop(&self, region: PixelRegion) -> Result<Option<Bytes>> {
        let Some(region) = self.clamp(region) else {
            return Ok(None);
        };
        let cropped = self
            .inner
            .crop_imm(region.x, region.y, region.width, region.height);
        Ok(Some(Self::encode_image(&cropped, self.format)?))
    }

    /// Paint `replacement` over `region` in place (blur, pixelate, block, or
    /// remove). A region that does not overlap the image is a silent no-op, and
    /// leaves the buffer clean; redaction acts on the in-bounds intersection.
    pub fn redact(&mut self, region: PixelRegion, replacement: &ImageReplacement) {
        let Some(region) = self.clamp(region) else {
            return;
        };
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

#[cfg(all(test, feature = "png"))]
mod tests {
    use image::{ImageFormat as ImgFormat, RgbImage};

    use super::*;

    /// An `w`x`h` solid-red PNG, as container bytes.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImgFormat::Png)
            .expect("encode png");
        bytes
    }

    /// The color at `(x, y)` of a decoded image.
    fn pixel_at(bytes: &[u8], x: u32, y: u32) -> Rgba<u8> {
        image::load_from_memory(bytes)
            .expect("decode")
            .get_pixel(x, y)
    }

    #[test]
    fn crop_wholly_outside_the_image_is_none() {
        let buffer = ImageBuffer::open(&png(4, 4)).expect("open");
        // A region past the right/bottom edges has no overlap with the image.
        assert!(
            buffer
                .crop(PixelRegion::new(10, 10, 4, 4))
                .expect("crop")
                .is_none()
        );
        // A zero-area region is likewise nothing to crop.
        assert!(
            buffer
                .crop(PixelRegion::new(0, 0, 0, 4))
                .expect("crop")
                .is_none()
        );
    }

    #[test]
    fn a_region_near_u32_max_does_not_overflow() {
        // The edge sum `x + width` must not overflow: a huge origin plus a huge
        // width has no overlap with the image and resolves to a clean `None`,
        // not a debug panic or a wrapped-around bogus region.
        let buffer = ImageBuffer::open(&png(4, 4)).expect("open");
        assert!(
            buffer
                .crop(PixelRegion::new(u32::MAX - 1, 0, 100, 4))
                .expect("crop")
                .is_none()
        );
        let mut buffer = buffer;
        buffer.redact(
            PixelRegion::new(u32::MAX - 1, u32::MAX - 1, u32::MAX, u32::MAX),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        assert!(
            !buffer.dirty,
            "an overflowing out-of-bounds redaction is a no-op"
        );
    }

    #[test]
    fn crop_partly_outside_is_clamped_to_the_overlap() {
        let buffer = ImageBuffer::open(&png(4, 4)).expect("open");
        // Starts inside, runs two pixels past each edge → a 2x2 overlap.
        let cropped = buffer
            .crop(PixelRegion::new(2, 2, 4, 4))
            .expect("crop")
            .expect("some overlap");
        let (w, h) = image::load_from_memory(&cropped)
            .expect("decode")
            .dimensions();
        assert_eq!((w, h), (2, 2));
    }

    #[test]
    fn redact_wholly_outside_is_a_clean_no_op() {
        let mut buffer = ImageBuffer::open(&png(4, 4)).expect("open");
        buffer.redact(
            PixelRegion::new(10, 10, 4, 4),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        // No overlap: nothing was painted, so the buffer stays clean and an
        // encode round-trips the source untouched.
        assert!(
            !buffer.dirty,
            "an out-of-bounds redaction must not dirty the buffer"
        );
    }

    #[test]
    fn redact_paints_only_the_in_bounds_intersection() {
        let mut buffer = ImageBuffer::open(&png(4, 4)).expect("open");
        // A 2x2 block starting at (3,3) reaches one pixel past each edge; only the
        // single in-bounds pixel (3,3) must turn black, and (0,0) stays red.
        buffer.redact(
            PixelRegion::new(3, 3, 2, 2),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        assert!(buffer.dirty);
        let out = encode_buffer(&buffer);
        assert_eq!(
            pixel_at(&out, 3, 3),
            Rgba([0, 0, 0, 255]),
            "target pixel not blacked"
        );
        assert_eq!(
            pixel_at(&out, 0, 0),
            Rgba([200, 30, 30, 255]),
            "untargeted pixel changed"
        );
    }

    /// Encode a redacted PNG buffer to bytes, feature-agnostic across the two
    /// `encode` signatures.
    fn encode_buffer(buffer: &ImageBuffer) -> Bytes {
        #[cfg(feature = "exif")]
        {
            buffer.encode(ExifPolicy::StripAll).expect("encode")
        }
        #[cfg(not(feature = "exif"))]
        {
            buffer.encode().expect("encode")
        }
    }

    #[test]
    fn coordinates_are_stored_pixel_space_not_display_space() {
        // The buffer decodes pixels as stored and does not apply EXIF
        // Orientation, so a redaction at a stored coordinate hits that stored
        // pixel regardless of any orientation a viewer would apply. A 6x2 image
        // (portrait-when-rotated) blacked at stored (0,0) has (0,0) black and the
        // far corner untouched, proving no implicit rotation moved the target.
        let mut buffer = ImageBuffer::open(&png(6, 2)).expect("open");
        assert_eq!(
            buffer.dimensions(),
            Dimensions::new(6, 2),
            "stored size, un-rotated"
        );
        buffer.redact(
            PixelRegion::new(0, 0, 1, 1),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = encode_buffer(&buffer);
        assert_eq!(pixel_at(&out, 0, 0), Rgba([0, 0, 0, 255]));
        assert_eq!(pixel_at(&out, 5, 1), Rgba([200, 30, 30, 255]));
    }

    /// After a pixel redaction, `StripSensitive` must retain benign metadata
    /// (Orientation) while dropping sensitive fields (GPS) — not throw away all
    /// EXIF with the re-encode, which the policy promises to preserve.
    #[cfg(all(feature = "jpeg", feature = "exif"))]
    #[test]
    fn strip_sensitive_after_redaction_keeps_benign_metadata() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        // A JPEG carrying a benign Orientation and a sensitive GPS latitude.
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30])))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImgFormat::Jpeg)
            .expect("encode jpeg");
        let mut exif = ExifMetadata::new();
        exif.set_tag(ExifTag::Orientation(vec![1]));
        exif.set_tag(ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
            nominator: 51,
            denominator: 1,
        }]));
        exif.write_to_vec(&mut bytes, FileExtension::JPEG)
            .expect("write exif");

        // Redact a pixel so the buffer is dirty (forces the re-encode path).
        let mut buffer = ImageBuffer::open(&bytes).expect("open");
        buffer.redact(
            PixelRegion::new(0, 0, 2, 2),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = buffer.encode(ExifPolicy::StripSensitive).expect("encode");

        // The GPS field is gone; the benign Orientation survives.
        let out_exif = ExifMetadata::new_from_vec(&out.to_vec(), FileExtension::JPEG)
            .expect("parse output exif");
        assert!(
            out_exif
                .get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_none(),
            "sensitive GPS survived a StripSensitive re-encode"
        );
        assert!(
            out_exif
                .get_tag(&ExifTag::Orientation(Vec::new()))
                .next()
                .is_some(),
            "benign Orientation was dropped by a StripSensitive re-encode"
        );
    }

    /// Metadata-only strip on a TIFF (pixels untouched): the lossless path reads
    /// the source TIFF's IFD, drops the sensitive tags, and writes the whole
    /// file back. TIFF stores EXIF as its own IFD, so this exercises the
    /// EXIF-as-IFD read-modify-write.
    #[cfg(all(feature = "tiff", feature = "exif"))]
    #[test]
    fn strip_gps_without_redaction_round_trips_tiff() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        let bytes = crate::test_util::tiff_with_gps();
        assert!(
            crate::test_util::has_gps_tiff(&bytes),
            "fixture should carry GPS before the strip"
        );

        // No pixel redaction: the lossless strip path.
        let buffer = ImageBuffer::open(&bytes).expect("open");
        let out = buffer.encode(ExifPolicy::StripSensitive).expect("encode");

        // Pixels intact, GPS gone.
        assert_eq!(
            pixel_at(&out, 0, 0),
            Rgba([200, 30, 30, 255]),
            "pixels changed"
        );
        assert!(
            ExifMetadata::new_from_vec(&out.to_vec(), FileExtension::TIFF)
                .expect("parse output")
                .get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_none(),
            "GPS survived the TIFF strip"
        );
    }

    /// Pixel redaction + metadata on a TIFF: the re-encode path. `little_exif`'s
    /// TIFF writer serializes a whole TIFF from the parsed metadata, so the
    /// transfer path transplants the source's kept tags onto the freshly-encoded
    /// (redacted) container. Both the pixel redaction and the kept EXIF must
    /// survive one encode.
    #[cfg(all(feature = "tiff", feature = "exif"))]
    #[test]
    fn redact_then_keep_metadata_tiff() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        let bytes = crate::test_util::tiff_with_gps();
        let mut buffer = ImageBuffer::open(&bytes).expect("open");
        buffer.redact(
            PixelRegion::new(0, 0, 2, 2),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = buffer.encode(ExifPolicy::Keep).expect("encode");
        // The pixel redaction survives.
        assert_eq!(
            pixel_at(&out, 0, 0),
            Rgba([0, 0, 0, 255]),
            "pixel not redacted"
        );
        assert_eq!(
            pixel_at(&out, 3, 3),
            Rgba([200, 30, 30, 255]),
            "untargeted pixel changed"
        );
        // And the kept EXIF (GPS, under Keep) is carried onto the redacted TIFF.
        assert!(
            ExifMetadata::new_from_vec(&out.to_vec(), FileExtension::TIFF)
                .expect("parse output")
                .get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_some(),
            "kept GPS was dropped on the TIFF transfer"
        );
    }

    /// Pixel redaction + `StripSensitive` on a TIFF, the leak-safety contract:
    /// after redacting pixels, every sensitive field (GPS in its sub-IFD, a
    /// device Make in IFD0) is gone, the redacted pixels survive, and a benign
    /// field (Orientation) is retained — proving the sub-IFD wholesale transfer
    /// and the IFD0 allowlist neither leak PII nor drop render-critical metadata.
    #[cfg(all(feature = "tiff", feature = "exif"))]
    #[test]
    fn redact_then_strip_sensitive_tiff() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        // A TIFF carrying GPS (sensitive, GPS sub-IFD), Make (sensitive, IFD0),
        // and Orientation (benign, IFD0).
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30])))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImgFormat::Tiff)
            .expect("encode tiff");
        let mut exif = ExifMetadata::new_from_vec(&bytes, FileExtension::TIFF).expect("parse ifd");
        exif.set_tag(ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
            nominator: 51,
            denominator: 1,
        }]));
        exif.set_tag(ExifTag::Make("Nvisy".into()));
        exif.set_tag(ExifTag::Orientation(vec![1]));
        exif.write_to_vec(&mut bytes, FileExtension::TIFF)
            .expect("write exif");

        let mut buffer = ImageBuffer::open(&bytes).expect("open");
        buffer.redact(
            PixelRegion::new(0, 0, 2, 2),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = buffer.encode(ExifPolicy::StripSensitive).expect("encode");
        let out_exif =
            ExifMetadata::new_from_vec(&out.to_vec(), FileExtension::TIFF).expect("parse output");
        let has = |tag: &ExifTag| out_exif.get_tag(tag).next().is_some();

        // Pixels redacted, and still a valid TIFF.
        assert_eq!(
            pixel_at(&out, 0, 0),
            Rgba([0, 0, 0, 255]),
            "pixel not redacted"
        );
        assert_eq!(
            pixel_at(&out, 3, 3),
            Rgba([200, 30, 30, 255]),
            "untargeted pixel changed"
        );
        // Every sensitive field is gone.
        assert!(
            !has(&ExifTag::GPSLatitude(Vec::new())),
            "sensitive GPS survived"
        );
        assert!(
            !has(&ExifTag::Make(String::new())),
            "sensitive Make survived"
        );
        // The benign field is retained.
        assert!(
            has(&ExifTag::Orientation(Vec::new())),
            "benign Orientation was dropped"
        );
    }
}
