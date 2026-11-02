//! [`Source`]: a borrowed image container plus its format, the receiver for the
//! EXIF read / strip / field-removal / transfer operations.

use std::panic::AssertUnwindSafe;

use elide_core::{Error, Result};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata as ExifMetadata;

use super::container::{Scrubbed, scrub_jpeg, scrub_png};
use super::guard;
use super::tags::{
    FIELD_KEYS, THUMBNAIL_TAGS, is_transferable_ifd0_tag, kept_as_content, render_tags,
    tags_for_key, written_tag,
};
use super::value::render;

/// The TIFF IFD0 tags that each hold a whole non-EXIF metadata block: XMP
/// (700), IPTC (33723), and Photoshop image resources (34377).
const TIFF_METADATA_BLOCK_TAGS: [u16; 3] = [0x02BC, 0x83BB, 0x8649];

/// A borrowed image container with its format, the receiver for the metadata
/// operations.
///
/// A thin lifetime wrapper over `&[u8]` so the bytes and the format that
/// interprets them travel together, and the read/strip/transfer operations read
/// as methods on the source rather than free functions taking a loose pair.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Source<'a> {
    bytes: &'a [u8],
    format: FileExtension,
}

impl<'a> Source<'a> {
    /// Wrap `bytes` interpreted as `format`.
    pub(crate) fn new(bytes: &'a [u8], format: FileExtension) -> Self {
        Self { bytes, format }
    }

    /// Parse the source's EXIF, or `None` when it carries none.
    fn parse(&self) -> Result<Option<ExifMetadata>> {
        let buffer = self.bytes.to_vec();
        let format = self.format;
        Ok(guard(|| ExifMetadata::new_from_vec(&buffer, format))?.ok())
    }

    /// The value of every surfaced field the image carries, as `(key, value)`
    /// in [`FIELD_KEYS`] order, each rendered as the text a reader would see.
    /// Empty when the image carries no EXIF (the common case for PNG).
    pub(crate) fn values(&self) -> Result<Vec<(&'static str, String)>> {
        let Some(exif) = self.parse()? else {
            return Ok(Vec::new());
        };
        Ok(FIELD_KEYS
            .into_iter()
            .filter_map(|key| {
                let value = guard(AssertUnwindSafe(|| render(key, &exif)))
                    .ok()
                    .flatten()?;
                Some((key, value))
            })
            .collect())
    }

    /// Whether the EXIF `exif` embeds a thumbnail: an image's IFD1 preview.
    /// A TIFF's embedded JPEG is its image data, never a thumbnail.
    fn has_thumbnail(&self, exif: &ExifMetadata) -> bool {
        !matches!(self.format, FileExtension::TIFF)
            && exif.get_ifds().iter().any(|ifd| {
                ifd.get_ifd_type() == ExifTagGroup::GENERIC
                    && ifd
                        .get_tags()
                        .iter()
                        .any(|tag| THUMBNAIL_TAGS.contains(&tag.as_u16()))
            })
    }

    /// Remove every metadata block but what a viewer needs to render the image
    /// (its orientation), returning the resulting container and how many blocks
    /// were removed.
    ///
    /// The whole container is scrubbed, not only its EXIF: XMP, IPTC, comments,
    /// and text chunks go too, so nothing the field table does not name can
    /// survive. The kept render tags are then written back as a minimal EXIF
    /// block.
    pub(crate) fn strip(&self) -> Result<Scrubbed> {
        let (kept, thumbnails): (Vec<ExifTag>, usize) = match self.parse()? {
            Some(exif) => guard(|| {
                let kept = render_tags()
                    .iter()
                    .filter_map(|tag| exif.get_tag(tag).next().cloned())
                    .collect();
                (kept, usize::from(self.has_thumbnail(&exif)))
            })?,
            None => (Vec::new(), 0),
        };
        let cleared = match self.format {
            FileExtension::JPEG => scrub_jpeg(self.bytes, false)?,
            FileExtension::PNG { .. } => scrub_png(self.bytes, false)?,
            // A TIFF's metadata is its own IFD: reduce it to the tags the
            // pixels need.
            FileExtension::TIFF => {
                let mut buf = self.bytes.to_vec();
                guard(|| ExifMetadata::clear_metadata(&mut buf, FileExtension::TIFF))?
                    .map_err(|e| Error::malformed(format!("exif strip: {e}")))?;
                let removed = usize::from(buf != self.bytes);
                Scrubbed {
                    bytes: buf,
                    removed,
                    thumbnails: 0,
                }
            }
            other => return Err(unsupported(other)),
        };
        let cleared = Scrubbed {
            thumbnails,
            ..cleared
        };
        if kept.is_empty() || cleared.removed == 0 {
            return Ok(cleared);
        }
        let source = Source::new(&cleared.bytes, self.format);
        let mut exif = source.parse()?.unwrap_or_else(ExifMetadata::new);
        guard(AssertUnwindSafe(|| {
            for tag in kept {
                exif.set_tag(tag);
            }
        }))?;
        Ok(Scrubbed {
            bytes: source.write(&exif)?,
            removed: cleared.removed,
            thumbnails,
        })
    }

    /// Reduce the metadata to what [`TreatAsContent`] can stand behind: every EXIF tag
    /// a surfaced field reads or clears, the render tag, the structure, and a
    /// TIFF's pixel layout. Every other EXIF tag (one no field surfaces, an
    /// embedded thumbnail, a maker note, a vendor tag) and every block outside
    /// EXIF is removed, since none of it can reach detection. Returns the
    /// resulting container, how many tags and blocks went, and how many
    /// thumbnails.
    ///
    /// [`TreatAsContent`]: elide_core::redaction::Disposition::TreatAsContent
    pub(crate) fn reduce_to_surfaced(&self) -> Result<Scrubbed> {
        let tiff = matches!(self.format, FileExtension::TIFF);
        let (edited, tags, thumbnails) = match self.parse()? {
            Some(mut exif) => {
                let thumbnails = usize::from(self.has_thumbnail(&exif));
                let unsurfaced = self.unsurfaced(&exif);
                if unsurfaced.is_empty() {
                    (self.bytes.to_vec(), 0, 0)
                } else {
                    let tags = unsurfaced.len();
                    guard(AssertUnwindSafe(|| {
                        for (hex, group) in unsurfaced {
                            exif.remove_tag_by_hex_group(hex, group);
                        }
                    }))?;
                    (self.write(&exif)?, tags, thumbnails)
                }
            }
            None => (self.bytes.to_vec(), 0, 0),
        };
        let mut scrubbed = match self.format {
            FileExtension::JPEG => scrub_jpeg(&edited, true)?,
            FileExtension::PNG { .. } => scrub_png(&edited, true)?,
            FileExtension::TIFF => Scrubbed {
                bytes: edited,
                removed: 0,
                thumbnails: 0,
            },
            other => return Err(unsupported(other)),
        };
        // A thumbnail's two tags are one thumbnail, not two stripped tags.
        let thumbnail_tags = if tiff {
            0
        } else {
            thumbnails * THUMBNAIL_TAGS.len()
        };
        scrubbed.removed += tags - thumbnail_tags.min(tags);
        scrubbed.thumbnails = thumbnails;
        Ok(scrubbed)
    }

    /// The `(tag, IFD group)` of every EXIF tag in `exif` a redacted image may
    /// not keep, each once.
    fn unsurfaced(&self, exif: &ExifMetadata) -> Vec<(u16, ExifTagGroup)> {
        let tiff = matches!(self.format, FileExtension::TIFF);
        let mut out: Vec<(u16, ExifTagGroup)> = exif
            .get_ifds()
            .iter()
            .flat_map(|ifd| {
                let group = ifd.get_ifd_type();
                ifd.get_tags().iter().map(move |tag| (tag.as_u16(), group))
            })
            .filter(|&(hex, group)| !kept_as_content(hex, group, tiff))
            .collect();
        out.sort_unstable_by_key(|&(hex, group)| (hex, group as u8));
        out.dedup();
        out
    }

    /// Remove the fields named by `removed` (each an EXIF tag key such as
    /// `"GPSLatitude"`) and overwrite those named by `replaced` with their new
    /// text, leaving all other EXIF fields intact, then scrub every metadata
    /// block outside EXIF, returning the resulting container and how many
    /// blocks outside EXIF were removed.
    ///
    /// Unlike [`strip`](Self::strip), which drops everything but the render
    /// tags, this edits only the caller's chosen keys, so a per-field
    /// redaction decision (from picked entities) maps straight to the byte edit.
    /// A field is cleared with its whole logical group (the GPS IFD for a
    /// coordinate, the lens tags for a device), and an overwritten field is
    /// then written back alone; a field that cannot hold text is only cleared.
    /// A key not present, or not one this crate maps to a tag, is skipped. The
    /// blocks outside EXIF (XMP, IPTC, comments, text chunks) cannot be
    /// addressed as fields, so they are always removed, even when nothing is
    /// edited.
    pub(crate) fn edit(&self, removed: &[&str], replaced: &[(&str, &str)]) -> Result<Scrubbed> {
        let cleared: Vec<ExifTag> = removed
            .iter()
            .chain(replaced.iter().map(|(key, _)| key))
            .flat_map(|key| tags_for_key(key))
            .collect();
        let written: Vec<ExifTag> = replaced
            .iter()
            .filter_map(|(key, value)| written_tag(key, value))
            .collect();
        let is_tiff = matches!(self.format, FileExtension::TIFF);
        let mut tiff_blocks = 0;
        let edited = match self.parse()? {
            Some(mut exif) if is_tiff || !cleared.is_empty() => {
                guard(AssertUnwindSafe(|| {
                    for tag in cleared {
                        exif.remove_tag(tag);
                    }
                    for tag in written {
                        exif.set_tag(tag);
                    }
                    // A TIFF keeps its non-EXIF metadata blocks as IFD0 tags.
                    if is_tiff {
                        for hex in TIFF_METADATA_BLOCK_TAGS {
                            let present = exif
                                .get_tag_by_hex(hex, Some(ExifTagGroup::GENERIC))
                                .next()
                                .is_some();
                            if present {
                                exif.remove_tag_by_hex_group(hex, ExifTagGroup::GENERIC);
                                tiff_blocks += 1;
                            }
                        }
                    }
                }))?;
                self.write(&exif)?
            }
            _ => self.bytes.to_vec(),
        };
        match self.format {
            FileExtension::JPEG => scrub_jpeg(&edited, true),
            FileExtension::PNG { .. } => scrub_png(&edited, true),
            FileExtension::TIFF => Ok(Scrubbed {
                bytes: edited,
                removed: tiff_blocks,
                thumbnails: 0,
            }),
            other => Err(unsupported(other)),
        }
    }

    /// Check that this container carries no metadata a decode-time strip would
    /// remove: no block outside EXIF and, unless `fields_allowed`, no surfaced
    /// EXIF field. The check run on an encoded image before it is returned.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) if metadata survived, or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the container cannot be read.
    #[cfg(feature = "codec")]
    pub(crate) fn check_stripped(&self, fields_allowed: bool) -> Result<()> {
        let outside_exif = match self.format {
            FileExtension::JPEG => scrub_jpeg(self.bytes, true)?.removed > 0,
            FileExtension::PNG { .. } => scrub_png(self.bytes, true)?.removed > 0,
            FileExtension::TIFF => self.parse()?.is_some_and(|exif| {
                TIFF_METADATA_BLOCK_TAGS.iter().any(|&hex| {
                    exif.get_tag_by_hex(hex, Some(ExifTagGroup::GENERIC))
                        .next()
                        .is_some()
                })
            }),
            other => return Err(unsupported(other)),
        };
        if outside_exif {
            return Err(survived("a metadata block outside EXIF"));
        }
        if fields_allowed {
            let unsurfaced = match self.parse()? {
                Some(exif) => guard(AssertUnwindSafe(|| self.unsurfaced(&exif)))?,
                None => Vec::new(),
            };
            if let Some((hex, _)) = unsurfaced.first() {
                return Err(survived(&format!("EXIF tag {hex:#06x} no field surfaces")));
            }
        } else if let Some((key, _)) = self.values()?.first() {
            return Err(survived(key));
        }
        Ok(())
    }

    /// Copy this source's full metadata onto `dest` (freshly re-encoded pixels
    /// that carry none), so the metadata a decode kept survives a pixel edit.
    pub(crate) fn transfer(&self, dest: Vec<u8>) -> Result<Vec<u8>> {
        let Some(exif) = self.parse()? else {
            // Source had no metadata: nothing to carry over.
            return Ok(dest);
        };
        // TIFF stores EXIF *as* the file's IFD, so `little_exif`'s TIFF writer
        // rebuilds the whole file from the metadata and ignores `dest`'s pixels
        // — the JPEG/PNG segment-layering below would silently drop the pixel
        // redactions. Instead transplant this source's non-structural tags onto
        // the freshly-encoded (redacted) container's own metadata, which already
        // carries the redacted pixels as its strip data.
        if matches!(self.format, FileExtension::TIFF) {
            return self.transfer_tiff(&exif, dest);
        }
        let format = self.format;
        let mut out = dest;
        guard(AssertUnwindSafe(|| exif.write_to_vec(&mut out, format)))?
            .map_err(|e| Error::malformed(format!("exif transfer: {e}")))?;
        Ok(out)
    }

    /// The TIFF path for [`transfer`](Self::transfer): parse the redacted
    /// container `dest` (whose metadata already holds the redacted pixels and its
    /// own correct pixel-layout tags) and copy `source`'s *metadata* tags onto
    /// it, so the output carries the redacted pixels plus the source's kept EXIF.
    ///
    /// The transfer is an allowlist, not a denylist: only tags in the EXIF, GPS,
    /// and Interop sub-IFDs (which are pure metadata) plus a fixed set of benign
    /// IFD0 descriptive tags are carried. Every pixel-layout tag — strips, tiles,
    /// planar config, dimensions, compression, and any tag `little_exif` parses
    /// as an `Unknown*` variant — is left as `dest`'s own value, so a source
    /// whose layout differs from the encoder's (e.g. a tiled source vs. the
    /// strip-based re-encode) can never point the output IFD at the wrong pixels.
    fn transfer_tiff(&self, source: &ExifMetadata, dest: Vec<u8>) -> Result<Vec<u8>> {
        let mut out = guard(|| ExifMetadata::new_from_vec(&dest, FileExtension::TIFF))?
            .map_err(|e| Error::malformed(format!("tiff reparse: {e}")))?;
        guard(AssertUnwindSafe(|| {
            for ifd in source.get_ifds() {
                let generic = ifd.get_ifd_type() == ExifTagGroup::GENERIC;
                for tag in ifd.get_tags() {
                    // Sub-IFD tags are all metadata; IFD0 mixes metadata with
                    // pixel layout, so there only known-safe descriptive tags
                    // cross.
                    if !generic || is_transferable_ifd0_tag(tag) {
                        out.set_tag(tag.clone());
                    }
                }
            }
        }))?;
        let mut buf = dest;
        guard(AssertUnwindSafe(|| {
            out.write_to_vec(&mut buf, FileExtension::TIFF)
        }))?
        .map_err(|e| Error::malformed(format!("tiff transfer: {e}")))?;
        Ok(buf)
    }

    /// Write `exif` back onto this source's bytes.
    fn write(&self, exif: &ExifMetadata) -> Result<Vec<u8>> {
        let format = self.format;
        let mut buf = self.bytes.to_vec();
        guard(AssertUnwindSafe(|| exif.write_to_vec(&mut buf, format)))?
            .map_err(|e| Error::malformed(format!("exif rewrite: {e}")))?;
        Ok(buf)
    }
}

/// The error for metadata, named by `what`, that survived into an encoded
/// image.
#[cfg(feature = "codec")]
fn survived(what: &str) -> Error {
    Error::sanitization(format!("encoded image still carries metadata: {what}"))
}

/// The error for a container format this crate opens no images as.
fn unsupported(format: FileExtension) -> Error {
    Error::capability_unavailable(format!(
        "image metadata edit: unsupported container {format:?}"
    ))
}

#[cfg(all(test, feature = "jpeg"))]
mod tests {
    use image::{ImageFormat as ImgFormat, RgbImage};
    use little_exif::rational::uR64;

    use super::*;

    /// A minimal JPEG carrying the given EXIF tags, as raw container bytes.
    fn jpeg_with(tags: Vec<ExifTag>) -> Vec<u8> {
        // A 2x2 solid image, encoded to JPEG: real container the parser accepts.
        let mut bytes = Vec::new();
        let img = RgbImage::from_pixel(2, 2, image::Rgb([200, 30, 30]));
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImgFormat::Jpeg)
            .expect("encode jpeg");

        let mut exif = ExifMetadata::new();
        for tag in tags {
            exif.set_tag(tag);
        }
        exif.write_to_vec(&mut bytes, FileExtension::JPEG)
            .expect("write exif");
        bytes
    }

    /// A JPEG with no EXIF at all.
    fn jpeg_bare() -> Vec<u8> {
        jpeg_with(Vec::new())
    }

    fn source(bytes: &[u8]) -> Source<'_> {
        Source::new(bytes, FileExtension::JPEG)
    }

    /// The surfaced field keys `bytes` carries.
    fn keys(bytes: &[u8]) -> Vec<&'static str> {
        source(bytes)
            .values()
            .expect("read")
            .into_iter()
            .map(|(key, _)| key)
            .collect()
    }

    /// Whether `bytes` carries `tag`, read straight from little_exif so no tag
    /// the field table omits can hide.
    fn carries(bytes: &[u8], tag: &ExifTag) -> bool {
        ExifMetadata::new_from_vec(&bytes.to_vec(), FileExtension::JPEG)
            .is_ok_and(|exif| exif.get_tag(tag).next().is_some())
    }

    #[test]
    fn values_surface_identity_time_and_free_text_fields() {
        let bytes = jpeg_with(vec![
            ExifTag::Make("Nvisy".into()),
            ExifTag::SerialNumber("SN-42".into()),
            ExifTag::DateTimeOriginal("2024:01:02 03:04:05".into()),
            ExifTag::Artist("Jane Roe".into()),
            ExifTag::ImageDescription("Jane at the clinic".into()),
        ]);
        let values = source(&bytes).values().expect("read");
        let value = |key| {
            values
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
        };
        assert!(value("Make").contains("Nvisy"));
        assert!(value("SerialNumber").contains("SN-42"));
        assert!(value("DateTimeOriginal").contains("2024"));
        assert!(value("Artist").contains("Jane Roe"));
        assert!(value("ImageDescription").contains("clinic"));
    }

    #[test]
    fn a_capture_time_falls_back_to_the_create_date() {
        let bytes = jpeg_with(vec![ExifTag::CreateDate("2023:05:06 07:08:09".into())]);
        let values = source(&bytes).values().expect("read");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, "DateTimeOriginal");
        assert!(values[0].1.contains("2023"));
    }

    #[test]
    fn values_are_empty_without_exif() {
        assert!(keys(&jpeg_bare()).is_empty());
    }

    #[test]
    fn strip_clears_every_field_but_keeps_the_orientation() {
        let bytes = jpeg_with(vec![
            ExifTag::Make("Nvisy".into()),
            ExifTag::Artist("Jane Roe".into()),
            ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
                nominator: 51,
                denominator: 1,
            }]),
            ExifTag::Orientation(vec![6]),
        ]);
        let stripped = source(&bytes).strip().expect("strip").bytes;
        assert!(keys(&stripped).is_empty(), "left: {:?}", keys(&stripped));
        assert!(carries(&stripped, &ExifTag::Orientation(Vec::new())));
    }

    #[test]
    fn strip_on_a_bare_image_is_a_successful_no_op() {
        let stripped = source(&jpeg_bare())
            .strip()
            .expect("strip a bare image")
            .bytes;
        assert!(keys(&stripped).is_empty());
        assert!(!carries(&stripped, &ExifTag::Orientation(Vec::new())));
    }

    /// The same bare-image strip on PNG: the chunk scrub must succeed rather
    /// than error on an image that carries no metadata.
    #[cfg(feature = "png")]
    #[test]
    fn strip_on_a_bare_png_is_a_successful_no_op() {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30])))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImgFormat::Png)
            .expect("encode png");
        let png = FileExtension::PNG {
            as_zTXt_chunk: false,
        };
        let stripped = Source::new(&bytes, png)
            .strip()
            .expect("strip a bare png")
            .bytes;
        assert!(
            Source::new(&stripped, png)
                .values()
                .expect("read")
                .is_empty()
        );
    }

    #[test]
    fn transfer_carries_metadata_onto_fresh_bytes() {
        let with_exif = jpeg_with(vec![ExifTag::Make("Nvisy".into())]);
        let merged = source(&with_exif).transfer(jpeg_bare()).expect("transfer");
        assert_eq!(keys(&merged), ["Make"]);
    }

    #[test]
    fn removing_a_free_text_key_clears_only_that_tag() {
        let bytes = jpeg_with(vec![
            ExifTag::Artist("Jane Roe".into()),
            ExifTag::Make("Nvisy".into()),
        ]);
        let stripped = source(&bytes).edit(&["Artist"], &[]).expect("strip").bytes;
        assert_eq!(keys(&stripped), ["Make"]);
    }

    #[test]
    fn overwriting_a_text_key_writes_the_new_value() {
        let bytes = jpeg_with(vec![
            ExifTag::Artist("Jane Roe".into()),
            ExifTag::Make("Nvisy".into()),
        ]);
        let edited = source(&bytes)
            .edit(&[], &[("Artist", "[REDACTED]")])
            .expect("edit")
            .bytes;
        let values = source(&edited).values().expect("values");
        assert!(values.contains(&("Artist", "[REDACTED]".to_owned())));
        assert!(values.contains(&("Make", "Nvisy".to_owned())));
    }

    #[test]
    fn overwriting_a_coordinate_removes_it() {
        let ur = |n| {
            vec![little_exif::rational::uR64 {
                nominator: n,
                denominator: 1,
            }]
        };
        let bytes = jpeg_with(vec![
            ExifTag::GPSLatitude(ur(51)),
            ExifTag::GPSLatitudeRef("N".into()),
        ]);
        let edited = source(&bytes)
            .edit(&[], &[("GPSLatitude", "[REDACTED]")])
            .expect("edit")
            .bytes;
        assert!(keys(&edited).is_empty());
    }

    /// Picking the surfaced `GPSLatitude` field removes the WHOLE GPS IFD, not
    /// just the latitude value: the hemisphere refs, altitude, GPS timestamp,
    /// and direction must all be gone, or a "geolocation" redaction still ships
    /// location-adjacent data.
    #[test]
    fn removing_a_gps_key_clears_the_whole_gps_ifd() {
        let ur = |n| {
            vec![little_exif::rational::uR64 {
                nominator: n,
                denominator: 1,
            }]
        };
        let bytes = jpeg_with(vec![
            ExifTag::GPSLatitude(ur(51)),
            ExifTag::GPSLatitudeRef("N".into()),
            ExifTag::GPSLongitude(ur(0)),
            ExifTag::GPSLongitudeRef("W".into()),
            ExifTag::GPSAltitude(ur(100)),
            ExifTag::GPSTimeStamp(ur(12)),
            ExifTag::GPSDateStamp("2024:01:02".into()),
        ]);
        // The picked key is the one the entity surfaces.
        let stripped = source(&bytes)
            .edit(&["GPSLatitude"], &[])
            .expect("strip")
            .bytes;

        // Every GPS tag must be gone, read straight from little_exif so no tag
        // the structured `Metadata` view omits can hide.
        let exif = ExifMetadata::new_from_vec(&stripped, FileExtension::JPEG).expect("parse");
        let present = |tag: &ExifTag| exif.get_tag(tag).next().is_some();
        assert!(
            !present(&ExifTag::GPSLatitude(Vec::new())),
            "latitude survived"
        );
        assert!(
            !present(&ExifTag::GPSLatitudeRef(String::new())),
            "lat ref survived"
        );
        assert!(
            !present(&ExifTag::GPSLongitude(Vec::new())),
            "longitude survived"
        );
        assert!(
            !present(&ExifTag::GPSLongitudeRef(String::new())),
            "lon ref survived"
        );
        assert!(
            !present(&ExifTag::GPSAltitude(Vec::new())),
            "altitude survived"
        );
        assert!(
            !present(&ExifTag::GPSTimeStamp(Vec::new())),
            "gps time survived"
        );
        assert!(
            !present(&ExifTag::GPSDateStamp(String::new())),
            "gps date survived"
        );
    }

    fn rational(nominator: u32) -> uR64 {
        uR64 {
            nominator,
            denominator: 1,
        }
    }

    /// Under `TreatAsContent` only what a surfaced field reads or clears stays:
    /// a maker note and a tag no field surfaces go, and the output check holds
    /// the reduced image to the same allowlist.
    #[cfg(feature = "codec")]
    #[test]
    fn treat_as_content_keeps_only_what_a_field_surfaces() {
        let bytes = jpeg_with(vec![
            ExifTag::Artist("Jane Roe".to_owned()),
            ExifTag::MakerNote(b"body serial 1234".to_vec()),
            ExifTag::ExposureTime(vec![uR64 {
                nominator: 1,
                denominator: 100,
            }]),
        ]);
        assert!(source(&bytes).check_stripped(true).is_err());

        let reduced = source(&bytes).reduce_to_surfaced().expect("reduce");
        assert_eq!(reduced.removed, 2);
        assert_eq!(keys(&reduced.bytes), ["Artist"]);
        assert!(!carries(&reduced.bytes, &ExifTag::MakerNote(Vec::new())));
        source(&reduced.bytes)
            .check_stripped(true)
            .expect("a reduced image passes");
    }

    /// An embedded thumbnail is a preview of the unredacted pixels:
    /// `TreatAsContent` strips it, counted as a thumbnail rather than as two tags.
    #[test]
    fn treat_as_content_strips_an_embedded_thumbnail() {
        let preview = b"\xFF\xD8 preview of Jane \xFF\xD9".to_vec();
        let bytes = jpeg_with(vec![
            ExifTag::Artist("Jane Roe".to_owned()),
            ExifTag::ThumbnailOffset(vec![0], preview.clone()),
            ExifTag::ThumbnailLength(vec![preview.len() as u32]),
        ]);
        assert!(contains(&bytes, b"preview of Jane"));

        let reduced = source(&bytes).reduce_to_surfaced().expect("reduce");
        assert_eq!((reduced.thumbnails, reduced.removed), (1, 0));
        assert!(!contains(&reduced.bytes, b"preview of Jane"));
        assert_eq!(keys(&reduced.bytes), ["Artist"]);
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn surfaced_values_read_as_text() {
        let bytes = jpeg_with(vec![
            ExifTag::Artist("Jane Roe".to_owned()),
            ExifTag::GPSLatitude(vec![rational(51), rational(30), rational(0)]),
            ExifTag::GPSLatitudeRef("N".to_owned()),
            ExifTag::UserComment(b"ASCII\0\0\0meet at noon".to_vec()),
        ]);
        let values = source(&bytes).values().expect("read");
        assert!(values.contains(&("GPSLatitude", "51.500000 N".to_owned())));
        assert!(values.contains(&("Artist", "Jane Roe".to_owned())));
        assert!(values.contains(&("UserComment", "meet at noon".to_owned())));
    }
}
