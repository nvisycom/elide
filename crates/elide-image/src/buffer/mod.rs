//! [`ImageBuffer`]: the single entry point into this crate.
//!
//! An opened image source — its decoded pixels plus its original container and
//! metadata — opened once, then read, redacted, and re-encoded.

mod raster;

use bytes::Bytes;
#[cfg(feature = "exif")]
use elide_core::modality::metadata::MetadataData;
use elide_core::{Error, Result};

pub use self::raster::RasterImage;

/// What a metadata strip removed from an image: how many metadata blocks and
/// tags, and how many embedded thumbnails.
#[cfg(feature = "exif")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StrippedMetadata {
    /// Metadata blocks (JPEG segments, PNG chunks) and EXIF or TIFF tags.
    pub properties: usize,
    /// Embedded thumbnails, previews of the unredacted pixels.
    pub thumbnails: usize,
}
#[cfg(feature = "exif")]
use crate::exif::{Scrubbed, Source};
use crate::modality::{ImageFormat, ImageReplacement};
use crate::primitive::{BoundingBox, Dimensions};

/// An opened image source, the single entry point into the crate.
///
/// [`open`](Self::open) ingests the bytes once, as the caller-resolved format:
/// it decodes the pixels into a [`RasterImage`] and retains the source container
/// so metadata is available without a second decode. Reuse the buffer to
/// [`redact`](Self::redact) pixel regions and [`encode`](Self::encode) back
/// out, all paying the decode cost a single time. With the `exif` feature it
/// also surfaces the source's privacy-relevant EXIF fields as labelled
/// [`MetadataData`] values, [edits](Self::edit_metadata) them, and
/// [strips](Self::strip_metadata) them.
///
/// The pixel work is the [`RasterImage`]'s; this type adds the source container
/// and metadata story on top. Cropping a buffer ([`crop`](Self::crop)) hands
/// back a bare `RasterImage` — a sub-image has no source of its own.
#[derive(Debug, Clone)]
pub struct ImageBuffer {
    /// The decoded pixels and their format; all pixel operations delegate here.
    raster: RasterImage,
    /// The source container bytes, kept so metadata (which the pixel decode
    /// discards) can be read or carried through on encode. The original bytes,
    /// or the metadata-stripped container once
    /// [`strip_metadata`](Self::strip_metadata) ran.
    source: Bytes,
    /// Whether a redaction has changed the pixels since `open`. When clean, an
    /// encode can operate on `source` directly (lossless); when dirty, the
    /// modified pixels must be re-encoded.
    dirty: bool,
}

impl ImageBuffer {
    /// Open `bytes` as the caller-resolved `format`: decode the pixels and keep
    /// the source.
    ///
    /// The format is not detected here — the ingestion registry already resolved
    /// it (from the extension, content type, or a magic-byte sniff) and routes
    /// to the matching codec; this decodes the bytes as that format.
    ///
    /// # Coordinate space
    ///
    /// The pixels are decoded as stored, with the EXIF `Orientation` tag *not*
    /// applied, so a coordinate is a raw stored-pixel coordinate, not a
    /// display-space one. This is deliberate: recognition reads these same stored
    /// pixels, so a region it reports and a region [`redact`](Self::redact) paints
    /// share one coordinate system and always line up. A caller that holds
    /// display-space coordinates (e.g. from a viewer that honours `Orientation`)
    /// must map them into stored space before calling in, or the wrong pixels are
    /// edited. [`dimensions`](Self::dimensions) likewise reports stored size.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable) if this build has no codec for
    /// `format`, or [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the bytes do not decode as
    /// it.
    pub fn open(bytes: &[u8], format: ImageFormat) -> Result<Self> {
        // The format is not detected here — the ingestion registry already
        // resolved it and routed to the matching codec; this decodes the bytes
        // as that format.
        if !format.can_decode() {
            return Err(Error::capability_unavailable(format!(
                "no codec for {format:?} in this build"
            )));
        }
        let inner = image::load_from_memory_with_format(bytes, format.to_image())
            .map_err(|e| Error::malformed(format!("image decode: {e}")))?;
        Ok(Self {
            raster: RasterImage::new(inner, format),
            source: Bytes::copy_from_slice(bytes),
            dirty: false,
        })
    }

    /// The image's pixel dimensions, read from the container header **without
    /// decoding the pixels**.
    ///
    /// A cheap probe for a caller that needs only the size (e.g. to scale a
    /// vision model's normalized boxes): it parses the header, not every pixel,
    /// and does not retain the source. Use [`open`](Self::open) when the pixels
    /// are needed too.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the bytes are not a readable image.
    pub fn dimensions_of(bytes: &[u8]) -> Result<Dimensions<u32>> {
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| Error::malformed(format!("unknown image: {e}")))?;
        let (w, h) = reader
            .into_dimensions()
            .map_err(|e| Error::malformed(format!("image header: {e}")))?;
        Ok(Dimensions::new(w, h))
    }

    /// The decoded pixels, for a caller that wants the raster directly.
    #[must_use]
    pub fn raster(&self) -> &RasterImage {
        &self.raster
    }

    /// The image's pixel dimensions.
    #[must_use]
    pub fn dimensions(&self) -> Dimensions<u32> {
        self.raster.dimensions()
    }

    /// The format the image was opened as, and re-encodes to.
    #[must_use]
    pub fn format(&self) -> ImageFormat {
        self.raster.format()
    }

    /// The source container as an EXIF [`Source`], the receiver for metadata ops.
    #[cfg(feature = "exif")]
    fn exif_source(&self) -> Source<'_> {
        Source::new(&self.source, self.format().to_exif())
    }

    /// Each privacy-relevant EXIF field as a [`MetadataData`] labelled with
    /// what it is, for a caller that streams fields (the codec's EXIF stream).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the metadata is present but unparseable.
    #[cfg(feature = "exif")]
    pub fn metadata_fields(&self) -> Result<Vec<MetadataData>> {
        self.exif_source().fields()
    }

    /// The source container bytes: the original, or the container as the
    /// metadata edits so far left it.
    #[cfg(feature = "exif")]
    #[must_use]
    pub fn source_bytes(&self) -> Bytes {
        self.source.clone()
    }

    /// Write each EXIF field's new value in place, a blank value removing the
    /// field, each with its whole logical group cleared first (the GPS IFD for
    /// a coordinate, the lens tags for a device). A field that cannot hold
    /// text, such as a coordinate or a timestamp, is removed instead of
    /// overwritten. Every metadata block outside EXIF is removed too. The
    /// pixels are untouched; a later [`encode`](Self::encode) carries the
    /// edited metadata.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a metadata edit fails.
    #[cfg(feature = "exif")]
    pub fn edit_metadata(&mut self, fields: &[(&str, &str)]) -> Result<()> {
        let edited = self.exif_source().edit(fields)?;
        self.source = Bytes::from(edited.bytes);
        Ok(())
    }

    /// Remove the source's metadata in place, keeping only what a viewer needs
    /// to render the image (its orientation), so every later read and encode
    /// sees the stripped container. Returns what was removed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the metadata edit fails.
    #[cfg(feature = "exif")]
    pub fn strip_metadata(&mut self) -> Result<StrippedMetadata> {
        let stripped = self.exif_source().strip()?;
        Ok(self.replace_source(stripped))
    }

    /// Remove, in place, the metadata no surfaced EXIF field stands for: every
    /// block outside EXIF (XMP, IPTC, comments, text chunks), every EXIF tag no
    /// field reads or clears (a maker note, vendor tags), and an embedded
    /// thumbnail. What stays is exactly what detection sees as fields, plus
    /// what the image needs to render. Returns what was removed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the metadata edit fails.
    #[cfg(feature = "exif")]
    pub fn strip_unsurfaced_metadata(&mut self) -> Result<StrippedMetadata> {
        let scrubbed = self.exif_source().reduce_to_surfaced()?;
        Ok(self.replace_source(scrubbed))
    }

    /// Replace the source container with the `scrubbed` one when it removed
    /// anything, returning what it removed.
    #[cfg(feature = "exif")]
    fn replace_source(&mut self, scrubbed: Scrubbed) -> StrippedMetadata {
        let stripped = StrippedMetadata {
            properties: scrubbed.removed,
            thumbnails: scrubbed.thumbnails,
        };
        if stripped != StrippedMetadata::default() {
            self.source = Bytes::from(scrubbed.bytes);
        }
        stripped
    }

    /// Re-encode the image, returning the container bytes with the source's
    /// metadata.
    ///
    /// When no redaction has been applied the source container is returned
    /// untouched (lossless). When pixels have been redacted, the modified image
    /// is re-encoded and the source's metadata, whatever
    /// [`strip_metadata`](Self::strip_metadata) left of it, is carried onto the
    /// fresh bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`](elide_core::ErrorKind::Processing) if the encoder rejects the image, or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the metadata transfer fails.
    #[cfg(feature = "exif")]
    pub fn encode(&self) -> Result<Bytes> {
        if !self.dirty {
            return Ok(self.source.clone());
        }
        let encoded = self.raster.encode_raw()?;
        Ok(self.exif_source().transfer(encoded.into())?.into())
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
        self.raster.encode_raw()
    }

    /// Crop the in-bounds intersection of `region` out as a bare
    /// [`RasterImage`], or `None` when the region does not overlap the image (out
    /// of bounds or zero-area). The crop carries no source or metadata.
    #[must_use]
    pub fn crop(&self, region: BoundingBox<u32>) -> Option<RasterImage> {
        self.raster.crop(region)
    }

    /// Paint `replacement` over `region` in place (blur, pixelate, block, or
    /// remove). A region that does not overlap the image is a silent no-op, and
    /// leaves the buffer clean; redaction acts on the in-bounds intersection.
    pub fn redact(&mut self, region: BoundingBox<u32>, replacement: &ImageReplacement) {
        if self.raster.redact(region, replacement) {
            self.dirty = true;
        }
    }
}

#[cfg(all(test, feature = "png"))]
mod tests {
    use image::{GenericImageView, ImageFormat as ImgFormat, RgbImage, Rgba};

    use super::*;
    use crate::modality::ImageReplacement;
    use crate::primitive::{Color, Point};

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
    fn dimensions_of_reads_the_header_without_decoding() {
        // The header probe reports the true pixel size without a full decode.
        let dims = ImageBuffer::dimensions_of(&png(7, 3)).expect("header dimensions");
        assert_eq!(dims, Dimensions::new(7, 3));
        // Non-image bytes are a malformed-input error, not a panic.
        assert!(ImageBuffer::dimensions_of(b"not an image").is_err());
    }

    #[test]
    fn crop_wholly_outside_the_image_is_none() {
        let buffer = ImageBuffer::open(&png(4, 4), ImageFormat::Png).expect("open");
        // A region past the right/bottom edges has no overlap with the image.
        assert!(
            buffer
                .crop(BoundingBox::from_origin(
                    Point::new(10, 10),
                    Dimensions::new(4, 4)
                ))
                .is_none()
        );
        // A zero-area region is likewise nothing to crop.
        assert!(
            buffer
                .crop(BoundingBox::from_origin(
                    Point::new(0, 0),
                    Dimensions::new(0, 4)
                ))
                .is_none()
        );
    }

    #[test]
    fn a_region_near_u32_max_does_not_overflow() {
        // The edge sum `x + width` must not overflow: a huge origin plus a huge
        // width has no overlap with the image and resolves to a clean `None`,
        // not a debug panic or a wrapped-around bogus region.
        let buffer = ImageBuffer::open(&png(4, 4), ImageFormat::Png).expect("open");
        assert!(
            buffer
                .crop(BoundingBox::from_origin(
                    Point::new(u32::MAX - 1, 0),
                    Dimensions::new(100, 4)
                ))
                .is_none()
        );
        let mut buffer = buffer;
        buffer.redact(
            BoundingBox::from_origin(
                Point::new(u32::MAX - 1, u32::MAX - 1),
                Dimensions::new(u32::MAX, u32::MAX),
            ),
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
        let buffer = ImageBuffer::open(&png(4, 4), ImageFormat::Png).expect("open");
        // Starts inside, runs two pixels past each edge → a 2x2 overlap.
        let cropped = buffer
            .crop(BoundingBox::from_origin(
                Point::new(2, 2),
                Dimensions::new(4, 4),
            ))
            .expect("some overlap");
        assert_eq!(cropped.dimensions(), Dimensions::new(2, 2));
    }

    #[test]
    fn redact_wholly_outside_is_a_clean_no_op() {
        let mut buffer = ImageBuffer::open(&png(4, 4), ImageFormat::Png).expect("open");
        buffer.redact(
            BoundingBox::from_origin(Point::new(10, 10), Dimensions::new(4, 4)),
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
        let mut buffer = ImageBuffer::open(&png(4, 4), ImageFormat::Png).expect("open");
        // A 2x2 block starting at (3,3) reaches one pixel past each edge; only the
        // single in-bounds pixel (3,3) must turn black, and (0,0) stays red.
        buffer.redact(
            BoundingBox::from_origin(Point::new(3, 3), Dimensions::new(2, 2)),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        assert!(buffer.dirty);
        let out = buffer.encode().expect("encode");
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

    #[test]
    fn coordinates_are_stored_pixel_space_not_display_space() {
        // The buffer decodes pixels as stored and does not apply EXIF
        // Orientation, so a redaction at a stored coordinate hits that stored
        // pixel regardless of any orientation a viewer would apply. A 6x2 image
        // (portrait-when-rotated) blacked at stored (0,0) has (0,0) black and the
        // far corner untouched, proving no implicit rotation moved the target.
        let mut buffer = ImageBuffer::open(&png(6, 2), ImageFormat::Png).expect("open");
        assert_eq!(
            buffer.dimensions(),
            Dimensions::new(6, 2),
            "stored size, un-rotated"
        );
        buffer.redact(
            BoundingBox::from_origin(Point::new(0, 0), Dimensions::new(1, 1)),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = buffer.encode().expect("encode");
        assert_eq!(pixel_at(&out, 0, 0), Rgba([0, 0, 0, 255]));
        assert_eq!(pixel_at(&out, 5, 1), Rgba([200, 30, 30, 255]));
    }

    /// A stripped then redacted image keeps the render-critical Orientation and
    /// drops everything else (GPS): the re-encode carries exactly what the strip
    /// left onto the redacted pixels.
    #[cfg(all(feature = "jpeg", feature = "exif"))]
    #[test]
    fn strip_then_redact_keeps_only_the_orientation() {
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

        let mut buffer = ImageBuffer::open(&bytes, ImageFormat::Jpeg).expect("open");
        assert!(buffer.strip_metadata().expect("strip").properties > 0);
        // Redact a pixel so the buffer is dirty (forces the re-encode path).
        buffer.redact(
            BoundingBox::from_origin(Point::new(0, 0), Dimensions::new(2, 2)),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = buffer.encode().expect("encode");

        let out_exif = ExifMetadata::new_from_vec(&out.to_vec(), FileExtension::JPEG)
            .expect("parse output exif");
        assert!(
            out_exif
                .get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_none(),
            "GPS survived the strip"
        );
        assert!(
            out_exif
                .get_tag(&ExifTag::Orientation(Vec::new()))
                .next()
                .is_some(),
            "Orientation was dropped by the strip"
        );
    }

    /// Metadata-only strip on a TIFF (pixels untouched): the lossless path clears
    /// the source TIFF's metadata and writes the whole file back. TIFF stores
    /// EXIF as its own IFD, so this exercises the EXIF-as-IFD read-modify-write.
    #[cfg(all(feature = "tiff", feature = "exif"))]
    #[test]
    fn strip_gps_without_redaction_round_trips_tiff() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        let bytes = crate::fixtures::tiff_with_gps();
        assert!(
            crate::fixtures::has_gps_tiff(&bytes),
            "fixture should carry GPS before the strip"
        );

        // No pixel redaction: the lossless strip path.
        let mut buffer = ImageBuffer::open(&bytes, ImageFormat::Tiff).expect("open");
        buffer.strip_metadata().expect("strip");
        let out = buffer.encode().expect("encode");

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

        let bytes = crate::fixtures::tiff_with_gps();
        let mut buffer = ImageBuffer::open(&bytes, ImageFormat::Tiff).expect("open");
        buffer.redact(
            BoundingBox::from_origin(Point::new(0, 0), Dimensions::new(2, 2)),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = buffer.encode().expect("encode");
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
        // And the unstripped EXIF (GPS) is carried onto the redacted TIFF.
        assert!(
            ExifMetadata::new_from_vec(&out.to_vec(), FileExtension::TIFF)
                .expect("parse output")
                .get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_some(),
            "kept GPS was dropped on the TIFF transfer"
        );
    }

    /// Metadata strip + pixel redaction on a TIFF, the leak-safety contract:
    /// every identifying field (GPS in its sub-IFD, a device Make in IFD0) is
    /// gone, the redacted pixels survive, and the render-critical Orientation is
    /// retained — proving the sub-IFD transfer and the IFD0 allowlist neither
    /// leak PII nor drop render-critical metadata.
    #[cfg(all(feature = "tiff", feature = "exif"))]
    #[test]
    fn strip_then_redact_tiff() {
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

        let mut buffer = ImageBuffer::open(&bytes, ImageFormat::Tiff).expect("open");
        buffer.strip_metadata().expect("strip");
        buffer.redact(
            BoundingBox::from_origin(Point::new(0, 0), Dimensions::new(2, 2)),
            &ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        let out = buffer.encode().expect("encode");
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
