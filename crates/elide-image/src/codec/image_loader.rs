//! [`ImageDocumentLoader`]: decode image bytes into a [`Document`]: the pixel
//! stream, plus the `#exif` metadata stream when metadata is treated as content.

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader, FormatId, StrippedContent};
use elide_core::Result;
use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

use super::exif_stream::ExifStream;
use super::image_document::ImageDocument;
use super::image_state::ImageState;
use super::image_stream::PixelStream;
use crate::ImageBuffer;
use crate::modality::ImageFormat;

/// A loader that decodes image bytes into a [`Document`]: the pixel stream, plus
/// the `#exif` metadata stream when the [`HiddenContentPolicy`] treats metadata
/// as content.
///
/// The policy's metadata disposition is applied here, before any part exists:
/// [`KeepOnlyEssential`](Disposition::KeepOnlyEssential) strips the source
/// container down to its render tags, and
/// [`ScanAsContent`](Disposition::ScanAsContent) strips everything no
/// surfaced field stands for and streams the surfaced EXIF fields.
pub struct ImageDocumentLoader {
    format_id: FormatId,
    format: ImageFormat,
}

impl ImageDocumentLoader {
    /// A loader for `format_id` / `format`.
    pub fn new(format_id: FormatId, format: ImageFormat) -> Self {
        Self { format_id, format }
    }
}

#[async_trait::async_trait]
impl DocumentLoader for ImageDocumentLoader {
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document> {
        // The registry routed to this loader for its one format; decode the
        // bytes as exactly that, no re-detection.
        let mut buffer = ImageBuffer::open(content.as_bytes(), self.format)?;
        let removed = match policy.metadata {
            Disposition::KeepOnlyEssential => buffer.strip_metadata()?,
            Disposition::ScanAsContent | Disposition::FlagEverything => {
                buffer.strip_unsurfaced_metadata()?
            }
        };
        let mut stripped = StrippedContent::new();
        stripped.record(HiddenContent::Properties, None, removed.properties);
        stripped.record(HiddenContent::Thumbnail, None, removed.thumbnails);
        let state = ImageState::new(buffer);

        // The `#exif` stream edits the same buffer as the pixel stream, so it
        // holds a handle to it.
        let exif = if policy.metadata.treats_as_content() {
            Some(ExifStream::new(state.clone(), self.format_id)?)
        } else {
            None
        };
        let native = ImageDocument {
            pixels: PixelStream {
                state,
                format_id: self.format_id,
            },
            exif,
            format: self.format,
            metadata: policy.metadata,
        };
        Ok(Document::new(self.format_id, native, Vec::new()).with_stripped(stripped))
    }
}

// `fixtures` provides the sample data below and implies `exif`, `jpeg`, and `png`,
// the decoders these tests exercise; without it (e.g. `--features codec` alone)
// the module would not compile.
#[cfg(all(test, feature = "fixtures"))]
mod tests {
    use bytes::Bytes;
    use elide_codec::TypedStream;
    use elide_core::modality::metadata::{Metadata, MetadataLocation};
    use elide_core::modality::text::TextReplacement;
    use elide_core::modality::{
        ChunkReader as _, DataReader as _, DataReaderExt as _, DataWriter as _,
    };
    use elide_core::redaction::Redactions;
    use image::GenericImageView;

    use super::*;
    use crate::codec::EXIF_PART_ID;
    use crate::fixtures;
    use crate::modality::{Image, ImageLocation, ImageReplacement};
    use crate::primitive::{BoundingBox, Color, Dimensions, Point};

    const JPEG: &str = "elide.image.jpeg";
    const PNG: &str = "elide.image.png";

    fn bbox(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    /// Decode `bytes` into an image [`Document`] under `format_id`, applying
    /// `metadata` to its metadata.
    async fn decode(format_id: &'static str, metadata: Disposition, bytes: Bytes) -> Document {
        let format = ImageFormat::detect(&bytes).expect("fixture is a valid image");
        let policy = HiddenContentPolicy::default().with_metadata(metadata);
        ImageDocumentLoader::new(FormatId::new(format_id), format)
            .decode(ContentData::new(bytes), policy)
            .await
            .expect("decode")
    }

    /// The document's pixel stream, as `Image`.
    fn pixels(doc: &mut Document) -> TypedStream<'_, Image> {
        let (_, stream) = doc.streams_mut().into_iter().next().expect("pixels");
        stream.downcast::<Image>().expect("pixel stream")
    }

    /// The document's `#exif` metadata stream, as `Metadata`.
    fn exif(doc: &mut Document) -> TypedStream<'_, Metadata> {
        doc.streams_mut()
            .into_iter()
            .find(|(id, _)| *id == EXIF_PART_ID)
            .and_then(|(_, stream)| stream.downcast::<Metadata>())
            .expect("#exif stream")
    }

    /// Redact the top-left 2x2 block of the pixel stream to black.
    async fn redact_top_left(doc: &mut Document) {
        pixels(doc)
            .write_at(Redactions::from([(
                bbox(0.0, 0.0, 2.0, 2.0),
                ImageReplacement::Block {
                    color: Color::BLACK,
                },
            )]))
            .await
            .unwrap();
    }

    /// Apply `replacement` to the field `key` of the document's `#exif` stream.
    async fn edit_field(doc: &mut Document, key: &str, replacement: TextReplacement) {
        exif(doc)
            .write_at(Redactions::from([(
                MetadataLocation::new(key.to_owned()),
                replacement,
            )]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn decode_stream_reports_full_frame() {
        let mut doc = decode(PNG, Disposition::KeepOnlyEssential, fixtures::png(4, 4)).await;
        assert_eq!(doc.streams_mut().len(), 1);
        let stream = pixels(&mut doc);
        assert_eq!(stream.format().as_str(), PNG);
        let chunks = stream.chunks().unwrap();
        // The stream yields exactly one full-frame chunk.
        assert_eq!(chunks.len(), 1);
        let dims = chunks[0].data.dimensions();
        assert_eq!((dims.width, dims.height), (4, 4));
    }

    #[tokio::test]
    async fn only_treating_metadata_as_content_surfaces_the_exif_stream() {
        let jpeg = fixtures::jpeg_with_gps;
        assert_eq!(
            decode(JPEG, Disposition::KeepOnlyEssential, jpeg())
                .await
                .streams_mut()
                .len(),
            1
        );
        assert_eq!(
            decode(JPEG, Disposition::ScanAsContent, jpeg())
                .await
                .streams_mut()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn read_at_crops_region() {
        let mut doc = decode(PNG, Disposition::KeepOnlyEssential, fixtures::png(4, 4)).await;
        let stream = pixels(&mut doc);
        let data = stream.expect_at(&bbox(1.0, 1.0, 2.0, 2.0)).await;
        let dims = data.dimensions();
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
        let mut doc = decode(PNG, Disposition::KeepOnlyEssential, fixtures::png(4, 4)).await;
        redact_top_left(&mut doc).await;
        let out = doc.encode().unwrap();
        let painted = image::load_from_memory(out.as_bytes()).unwrap();
        // Top-left corner is now black; an untouched corner keeps its color.
        assert_eq!(painted.get_pixel(0, 0), image::Rgba([0, 0, 0, 255]));
        assert_ne!(painted.get_pixel(3, 3), image::Rgba([0, 0, 0, 255]));
    }

    /// Stripping metadata happens at decode: the document is marked sanitized and
    /// its output carries no GPS even when nothing is redacted.
    #[tokio::test]
    async fn strip_removes_exif_at_decode() {
        let doc = decode(
            PNG,
            Disposition::KeepOnlyEssential,
            fixtures::png_with_gps(),
        )
        .await;
        assert!(doc.changed_at_decode());
        let out = doc.encode().unwrap();
        assert!(
            !fixtures::has_gps_png(out.as_bytes()),
            "GPS survived KeepOnlyEssential"
        );
    }

    /// Under `ScanAsContent` a jpeg decodes into a pixel stream and an `#exif`
    /// stream sharing one buffer, so redacting both yields one image carrying
    /// both edits.
    #[tokio::test]
    async fn image_document_composes_pixel_redaction_and_exif_removal() {
        let mut doc = decode(JPEG, Disposition::ScanAsContent, fixtures::jpeg_with_gps()).await;

        redact_top_left(&mut doc).await;
        edit_field(&mut doc, "GPSLatitude", TextReplacement::Removed).await;
        let out = doc.encode().unwrap();

        // Both edits landed: the corner is dark and the GPS tag is gone.
        let painted = image::load_from_memory(out.as_bytes()).unwrap();
        let redacted = painted.get_pixel(0, 0);
        assert!(
            redacted[0] < 60 && redacted[1] < 60 && redacted[2] < 60,
            "pixel redaction lost: {redacted:?}"
        );
        assert!(!fixtures::has_gps(out.as_bytes()), "GPS survived");
    }

    /// The `#exif` stream yields the surfaced fields, each labelled.
    #[tokio::test]
    async fn the_exif_stream_yields_labelled_fields() {
        let mut doc = decode(JPEG, Disposition::ScanAsContent, fixtures::jpeg_with_gps()).await;
        let chunks = exif(&mut doc).chunks().unwrap();
        assert!(chunks.iter().any(|chunk| chunk.data.key == "GPSLatitude"));
    }

    /// Under `ScanAsContent` a field nothing redacts stays.
    #[tokio::test]
    async fn scan_as_content_keeps_an_unflagged_field() {
        let mut doc = decode(PNG, Disposition::ScanAsContent, fixtures::png_with_gps()).await;
        redact_top_left(&mut doc).await;
        let out = doc.encode().unwrap();
        assert!(
            fixtures::has_gps_png(out.as_bytes()),
            "unflagged GPS dropped"
        );
    }
}
