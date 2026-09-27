//! The image codec on the parts model: a raster image is a [`Document`] of two
//! parts — the pixel [`Stream`] and the `#exif` [`Blob`] (the image's own bytes,
//! re-read as the `Metadata` modality) — recombined by laying the redacted
//! pixels over the metadata-stripped container.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{
    Document, DocumentLoader, DocumentPart, EncodedPart, ErasedStream, FormatId, LocalId,
    Recombine, Stream,
};
use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::exif_handler::EXIF_HINT;
use crate::ImageBuffer;
use crate::exif::ExifPolicy;
use crate::modality::{Image, ImageData, ImageLocation};
use crate::primitive::{BoundingBox, Dimensions, Point};

/// The `#exif` sub-part id: the image's own bytes, re-read as `Metadata`.
const EXIF_PART_ID: &str = "#exif";

/// The pixel body part id: the decoded (redacted) image.
const PIXEL_PART_ID: &str = "pixels";

/// The decoded image, shared between the pixel [`PixelStream`] and the
/// [`ImageRecombine`] so a redaction on the stream is visible when the
/// recombiner re-encodes. `Clone` shares the one buffer (an `Arc` bump); the
/// lock is held only inside these methods.
#[derive(Clone)]
struct ImageState(Arc<Mutex<ImageBuffer>>);

impl ImageState {
    /// Wrap a decoded image buffer.
    fn new(buffer: ImageBuffer) -> Self {
        Self(Arc::new(Mutex::new(buffer)))
    }

    /// The image's pixel dimensions.
    fn dimensions(&self) -> Dimensions<u32> {
        self.0.lock().unwrap().dimensions()
    }

    /// Encode the current pixels to bytes, applying `policy` to the metadata.
    fn encode(&self, policy: ExifPolicy) -> Result<Bytes> {
        self.0.lock().unwrap().encode(policy)
    }

    /// Encode the current pixels laid over the metadata-stripped `container`.
    fn encode_over_metadata(&self, container: &[u8]) -> Result<Bytes> {
        self.0.lock().unwrap().encode_over_metadata(container)
    }

    /// Crop the region `location` addresses and encode it, or `None` when the
    /// region falls outside the image.
    fn crop_encode(&self, location: &ImageLocation) -> Result<Option<ImageData>> {
        let buffer = self.0.lock().unwrap();
        let Some(region) = location.bounding_box.to_pixels(buffer.dimensions()) else {
            return Ok(None);
        };
        buffer
            .crop(region)
            .map(|raster| raster.encode())
            .transpose()
    }

    /// Redact every region in `redactions` that intersects the image, in place.
    fn redact(&self, redactions: Redactions<Image>) {
        let mut buffer = self.0.lock().unwrap();
        let dims = buffer.dimensions();
        for (location, replacement) in redactions.into_iter() {
            if let Some(region) = location.bounding_box.to_pixels(dims) {
                buffer.redact(region, &replacement);
            }
        }
    }
}

/// The pixel stream part: reads the whole frame as one chunk, redacts regions in
/// place on the shared [`ImageState`], and re-encodes just the pixels (its
/// metadata is handled by the `#exif` blob and the recombiner).
struct PixelStream {
    state: ImageState,
    format_id: FormatId,
    policy: ExifPolicy,
    yielded: bool,
}

#[async_trait::async_trait]
impl Stream<Image> for PixelStream {
    fn format(&self) -> FormatId {
        self.format_id.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::new(self.state.encode(self.policy)?))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Image>>> {
        if self.yielded {
            return Ok(None);
        }
        let dims = self.state.dimensions();
        let bbox = BoundingBox::from_origin(
            Point::new(0.0, 0.0),
            Dimensions::new(dims.width as f64, dims.height as f64),
        );
        let data = ImageData::new(self.state.encode(self.policy)?);
        self.yielded = true;
        Ok(Some(Chunk {
            location: ImageLocation::new(bbox),
            data,
            hints: Vec::new(),
        }))
    }
}

#[async_trait::async_trait]
impl DataReader<Image> for PixelStream {
    async fn read_at(&self, location: &ImageLocation) -> Result<Option<ImageData>> {
        self.state.crop_encode(location)
    }
}

#[async_trait::async_trait]
impl DataWriter<Image> for PixelStream {
    async fn write_at(&mut self, redactions: Redactions<Image>) -> Result<()> {
        self.state.redact(redactions);
        Ok(())
    }
}

/// The recombiner: fold the redacted pixels (from the shared buffer) and the
/// `#exif` blob into one image.
///
/// The `#exif` blob is the image's own bytes at decode. When a metadata pipeline
/// redacted it, its bytes differ from `original_exif`, so the redacted pixels are
/// laid over that metadata-stripped container (`encode_over_metadata`). When the
/// blob is untouched — its bytes still equal `original_exif` — no metadata
/// pipeline ran, so the fallback [`ExifPolicy`] governs the metadata instead.
struct ImageRecombine {
    state: ImageState,
    policy: ExifPolicy,
    /// The `#exif` blob's bytes at decode, to detect whether it was redacted.
    original_exif: Bytes,
}

impl Recombine for ImageRecombine {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        // A metadata pipeline redacts the `#exif` blob in place, changing its
        // bytes; only then does its container matter here.
        let exif = parts.iter().find(|p| p.id.as_str() == EXIF_PART_ID);
        let bytes = if exif.is_some_and(|p| p.bytes != self.original_exif) {
            // A metadata pipeline stripped the `#exif` container; lay the redacted
            // pixels over it. The pixel part's own encoding used the fallback
            // policy, so it can't be reused: the pixels must be re-laid over this
            // stripped container instead.
            let container = &exif.expect("checked present").bytes;
            self.state.encode_over_metadata(container)?
        } else {
            // Untouched metadata: the fallback policy governs it, which is exactly
            // how `PixelStream::encode` already encoded the pixel body part. Reuse
            // those bytes rather than encoding the image a second time.
            parts
                .iter()
                .find(|p| p.id.as_str() == PIXEL_PART_ID)
                .map(|p| p.bytes.clone())
                .map(Ok)
                .unwrap_or_else(|| self.state.encode(self.policy))?
        };
        Ok(ContentData::new(bytes))
    }
}

/// A loader that decodes image bytes into a two-part [`Document`]: the pixel
/// stream and the `#exif` metadata blob.
pub struct ImageDocumentLoader {
    format_id: FormatId,
    policy: ExifPolicy,
}

impl ImageDocumentLoader {
    /// A loader for `format_id` with the fallback `policy`.
    pub fn new(format_id: FormatId, policy: ExifPolicy) -> Self {
        Self { format_id, policy }
    }
}

#[async_trait::async_trait]
impl DocumentLoader for ImageDocumentLoader {
    async fn decode(&self, content: ContentData) -> Result<Document> {
        let buffer = ImageBuffer::open(content.as_bytes())?;
        let exif_bytes = buffer.source_bytes();
        let original_exif = exif_bytes.clone();
        let state = ImageState::new(buffer);

        let pixels = PixelStream {
            state: state.clone(),
            format_id: self.format_id.clone(),
            policy: self.policy,
            yielded: false,
        };
        let pixel_stream: Box<dyn Stream<Image>> = Box::new(pixels);

        Ok(Document::new(
            self.format_id.clone(),
            vec![
                DocumentPart::Stream {
                    id: LocalId::new("pixels"),
                    handle: ErasedStream::new(self.format_id.clone(), pixel_stream),
                },
                DocumentPart::Blob {
                    id: LocalId::new(EXIF_PART_ID),
                    bytes: exif_bytes,
                    hint: EXIF_HINT.to_owned(),
                },
            ],
            Box::new(ImageRecombine {
                state,
                policy: self.policy,
                original_exif,
            }),
        ))
    }
}

// `test-util` provides the fixtures below and implies `exif`, `jpeg`, and `png`,
// the decoders these tests exercise; without it (e.g. `--features codec` alone)
// the module would not compile.
#[cfg(all(test, feature = "test-util"))]
mod tests {
    use elide_codec::{DocumentLoader as _, LeafLoader};
    use elide_core::modality::StreamDataReader as _;
    use elide_core::modality::metadata::{Metadata, MetadataLocation, MetadataReplacement};
    use elide_core::redaction::Redactions;
    use image::GenericImageView;

    use super::super::exif_handler::ExifLoader;
    use super::*;
    use crate::exif::ExifPolicy;
    use crate::modality::ImageReplacement;
    use crate::primitive::Color;
    use crate::{ImageBuffer, test_util};

    const JPEG: &str = "elide.image.jpeg";
    const PNG: &str = "elide.image.png";

    fn bbox(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    /// Decode `bytes` into an image [`Document`] under `format_id` and `policy`.
    async fn decode(format_id: &'static str, policy: ExifPolicy, bytes: Bytes) -> Document {
        ImageDocumentLoader::new(FormatId::new(format_id), policy)
            .decode(ContentData::new(bytes))
            .await
            .expect("decode")
    }

    /// The document's pixel stream part, downcast to `Image`.
    fn pixels(doc: &mut Document) -> &mut elide_codec::TypedStream<Image> {
        match &mut doc.parts_mut()[0] {
            DocumentPart::Stream { handle, .. } => {
                handle.downcast_mut::<Image>().expect("pixel stream")
            }
            _ => panic!("part 0 is the pixel stream"),
        }
    }

    /// The document's `#exif` blob bytes.
    fn exif_bytes(doc: &Document) -> Bytes {
        doc.parts()
            .iter()
            .find_map(|p| match p {
                DocumentPart::Blob { id, bytes, .. } if id.as_str() == EXIF_PART_ID => {
                    Some(bytes.clone())
                }
                _ => None,
            })
            .expect("#exif blob")
    }

    /// Redact the top-left 2x2 block of the pixel stream to black.
    async fn redact_top_left(doc: &mut Document) {
        let mut batch: Redactions<Image> = Redactions::new();
        batch.push(
            bbox(0.0, 0.0, 2.0, 2.0),
            ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        pixels(doc).write_at(batch).await.unwrap();
    }

    /// Strip the GPS latitude from `exif_bytes` through the metadata pipeline,
    /// returning the metadata-stripped container bytes.
    async fn strip_gps(exif_bytes: Bytes) -> Bytes {
        let mut meta_doc = LeafLoader(ExifLoader)
            .decode(ContentData::new(exif_bytes))
            .await
            .unwrap();
        let DocumentPart::Stream { handle, .. } = &mut meta_doc.parts_mut()[0] else {
            panic!("exif is a stream")
        };
        let meta = handle.downcast_mut::<Metadata>().unwrap();
        let mut mb: Redactions<Metadata> = Redactions::new();
        mb.push(
            MetadataLocation::new("GPSLatitude"),
            MetadataReplacement::Removed,
        );
        meta.write_at(mb).await.unwrap();
        meta_doc.encode().unwrap().into_bytes()
    }

    #[tokio::test]
    async fn decode_stream_reports_full_frame() {
        let mut doc = decode(PNG, ExifPolicy::default(), test_util::png(4, 4)).await;
        assert_eq!(doc.parts().len(), 2);
        let stream = pixels(&mut doc);
        assert_eq!(stream.format().as_str(), PNG);
        let chunk = stream.read_next().await.unwrap().expect("one chunk");
        let dims = ImageBuffer::open(&chunk.data.bytes).unwrap().dimensions();
        assert_eq!((dims.width, dims.height), (4, 4));
        // The stream yields exactly one full-frame chunk.
        assert!(stream.read_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn read_at_crops_region() {
        let mut doc = decode(PNG, ExifPolicy::default(), test_util::png(4, 4)).await;
        let stream = pixels(&mut doc);
        let data = stream
            .read_at(&bbox(1.0, 1.0, 2.0, 2.0))
            .await
            .unwrap()
            .expect("crop");
        let dims = ImageBuffer::open(&data.bytes).unwrap().dimensions();
        assert_eq!((dims.width, dims.height), (2, 2));
        // An off-image region reads nothing.
        assert!(
            stream
                .read_at(&bbox(99.0, 99.0, 2.0, 2.0))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn redact_block_paints_region_and_reencodes() {
        let mut doc = decode(PNG, ExifPolicy::default(), test_util::png(4, 4)).await;
        redact_top_left(&mut doc).await;
        let out = doc.encode().unwrap();
        let painted = image::load_from_memory(out.as_bytes()).unwrap();
        // Top-left corner is now black; an untouched corner keeps its color.
        assert_eq!(painted.get_pixel(0, 0), image::Rgba([0, 0, 0, 255]));
        assert_ne!(painted.get_pixel(3, 3), image::Rgba([0, 0, 0, 255]));
    }

    /// The self-overlap composition: a jpeg decodes into a two-part Document
    /// (pixel stream + `#exif` blob). Redacting the pixel stream AND the exif
    /// blob (through its own decoded metadata document), then folding the blob
    /// back and encoding, yields ONE image carrying both edits.
    #[tokio::test]
    async fn image_document_composes_pixel_redaction_and_exif_strip() {
        let mut doc = decode(JPEG, ExifPolicy::default(), test_util::jpeg_with_gps()).await;

        redact_top_left(&mut doc).await;
        let stripped = strip_gps(exif_bytes(&doc)).await;
        doc.replace_part(&LocalId::new(EXIF_PART_ID), stripped)
            .unwrap();
        let out = doc.encode().unwrap();

        // Both edits landed: the corner is dark and the GPS tag is gone.
        let painted = image::load_from_memory(out.as_bytes()).unwrap();
        let redacted = painted.get_pixel(0, 0);
        assert!(
            redacted[0] < 60 && redacted[1] < 60 && redacted[2] < 60,
            "pixel redaction lost: {redacted:?}"
        );
        assert!(!test_util::has_gps(out.as_bytes()), "GPS survived");
    }

    /// Without a metadata pipeline the fallback [`ExifPolicy`] governs the output.
    /// The registered default strips EXIF (the privacy default); `Retain` keeps it.
    #[tokio::test]
    async fn fallback_policy_strips_or_keeps_exif() {
        // Default policy: EXIF is stripped.
        let mut default = decode(PNG, ExifPolicy::default(), test_util::png_with_gps()).await;
        assert!(
            test_util::has_gps_png(&exif_bytes(&default)),
            "fixture should carry GPS"
        );
        redact_top_left(&mut default).await;
        let out = default.encode().unwrap();
        assert!(
            !test_util::has_gps_png(out.as_bytes()),
            "default kept EXIF (should strip)"
        );

        // Retain policy: EXIF survives.
        let mut retain = decode(PNG, ExifPolicy::Retain, test_util::png_with_gps()).await;
        redact_top_left(&mut retain).await;
        let out = retain.encode().unwrap();
        assert!(
            test_util::has_gps_png(out.as_bytes()),
            "Retain policy dropped EXIF (should keep)"
        );
    }

    /// When the `#exif` blob IS redacted, its metadata-stripped result wins and
    /// the fallback policy is ignored — even a `Retain` document emits stripped
    /// metadata. Locks in the scope boundary of the policy knob.
    #[tokio::test]
    async fn exif_redaction_overrides_the_fallback_policy() {
        // A Retain document (would preserve EXIF on the un-redacted path)...
        let mut doc = decode(PNG, ExifPolicy::Retain, test_util::png_with_gps()).await;
        // ...but redact the `#exif` blob to strip GPS.
        let stripped = strip_gps(exif_bytes(&doc)).await;
        doc.replace_part(&LocalId::new(EXIF_PART_ID), stripped)
            .unwrap();
        let out = doc.encode().unwrap();
        assert!(
            !test_util::has_gps_png(out.as_bytes()),
            "Retain policy leaked past the #exif strip"
        );
    }
}
