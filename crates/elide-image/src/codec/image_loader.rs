//! [`ImageDocumentLoader`]: decode image bytes into a [`Document`]: the pixel
//! stream, plus the `#exif` metadata blob when metadata is redacted.

use std::iter;

use elide_codec::content::ContentData;
use elide_codec::{
    Document, DocumentLoader, DocumentPart, ErasedStream, FormatId, LocalId, Stream,
    StrippedContent,
};
use elide_core::Result;
use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

use super::exif_handler::EXIF_HINT;
use super::image_recombine::ImageRecombine;
use super::image_state::ImageState;
use super::image_stream::PixelStream;
use super::{EXIF_PART_ID, PIXEL_PART_ID};
use crate::ImageBuffer;
use crate::modality::{Image, ImageFormat};

/// A loader that decodes image bytes into a [`Document`]: the pixel stream, plus
/// the `#exif` metadata blob when the [`HiddenContentPolicy`] redacts metadata.
///
/// The policy's metadata disposition is applied here, before any part exists:
/// [`Strip`](Disposition::Strip) strips the source container down to its
/// render tags, and [`Redact`](Disposition::Redact) strips the blocks outside
/// EXIF and surfaces the EXIF fields as the `#exif` blob.
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
        let (blocks, exif) = match policy.metadata {
            Disposition::Strip => (buffer.strip_metadata()?, None),
            Disposition::Redact => {
                let blocks = buffer.strip_unsurfaced_metadata()?;
                let exif = DocumentPart::Blob {
                    id: LocalId::new(EXIF_PART_ID),
                    bytes: buffer.source_bytes(),
                    hint: EXIF_HINT.to_owned(),
                };
                (blocks, Some(exif))
            }
        };
        let mut stripped = StrippedContent::new();
        stripped.record(HiddenContent::Properties, None, blocks);
        let state = ImageState::new(buffer);

        let pixels: Box<dyn Stream<Image>> = Box::new(PixelStream {
            state: state.clone(),
            format_id: self.format_id.clone(),
        });
        let parts = iter::once(DocumentPart::Stream {
            id: LocalId::new(PIXEL_PART_ID),
            handle: ErasedStream::new(self.format_id.clone(), pixels),
        })
        .chain(exif)
        .collect();

        let recombine = ImageRecombine {
            state,
            format: self.format,
            metadata: policy.metadata,
        };
        Ok(
            Document::new(self.format_id.clone(), parts, Box::new(recombine))
                .with_stripped(stripped),
        )
    }
}

// `fixtures` provides the sample data below and implies `exif`, `jpeg`, and `png`,
// the decoders these tests exercise; without it (e.g. `--features codec` alone)
// the module would not compile.
#[cfg(all(test, feature = "fixtures"))]
mod tests {
    use bytes::Bytes;
    use elide_codec::{DocumentLoader as _, LeafLoader};
    use elide_core::modality::metadata::{Metadata, MetadataLocation, MetadataReplacement};
    use elide_core::modality::{DataReader as _, DataWriter as _, StreamDataReader as _};
    use elide_core::redaction::Redactions;
    use image::GenericImageView;

    use super::super::exif_handler::ExifLoader;
    use super::*;
    use crate::fixtures;
    use crate::modality::{ImageLocation, ImageReplacement};
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
            .decode(ContentData::new(exif_bytes), HiddenContentPolicy::default())
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
        let mut doc = decode(PNG, Disposition::Strip, fixtures::png(4, 4)).await;
        assert_eq!(doc.parts().len(), 1);
        let stream = pixels(&mut doc);
        assert_eq!(stream.format().as_str(), PNG);
        let chunks = stream.chunks().unwrap();
        // The stream yields exactly one full-frame chunk.
        assert_eq!(chunks.len(), 1);
        let dims = chunks[0].data.dimensions();
        assert_eq!((dims.width, dims.height), (4, 4));
    }

    #[tokio::test]
    async fn only_redacting_metadata_surfaces_the_exif_blob() {
        let jpeg = fixtures::jpeg_with_gps;
        assert_eq!(
            decode(JPEG, Disposition::Strip, jpeg()).await.parts().len(),
            1
        );
        assert_eq!(
            decode(JPEG, Disposition::Redact, jpeg())
                .await
                .parts()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn read_at_crops_region() {
        let mut doc = decode(PNG, Disposition::Strip, fixtures::png(4, 4)).await;
        let stream = pixels(&mut doc);
        let data = stream
            .read_at(&bbox(1.0, 1.0, 2.0, 2.0))
            .await
            .unwrap()
            .expect("crop");
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
        let mut doc = decode(PNG, Disposition::Strip, fixtures::png(4, 4)).await;
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
        let doc = decode(PNG, Disposition::Strip, fixtures::png_with_gps()).await;
        assert!(doc.changed_at_decode());
        let out = doc.encode().unwrap();
        assert!(!fixtures::has_gps_png(out.as_bytes()), "GPS survived Strip");
    }

    /// The self-overlap composition: under `Redact` a jpeg decodes into a
    /// two-part Document (pixel stream + `#exif` blob). Redacting the pixel
    /// stream AND the exif blob (through its own decoded metadata document), then
    /// folding the blob back and encoding, yields ONE image carrying both edits.
    #[tokio::test]
    async fn image_document_composes_pixel_redaction_and_exif_strip() {
        let mut doc = decode(JPEG, Disposition::Redact, fixtures::jpeg_with_gps()).await;

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
        assert!(!fixtures::has_gps(out.as_bytes()), "GPS survived");
    }

    /// Under `Redact` a field detection did not flag stays: the unedited `#exif`
    /// blob's fields carry through.
    #[tokio::test]
    async fn redact_keeps_an_unflagged_field() {
        let mut doc = decode(PNG, Disposition::Redact, fixtures::png_with_gps()).await;
        redact_top_left(&mut doc).await;
        let out = doc.encode().unwrap();
        assert!(
            fixtures::has_gps_png(out.as_bytes()),
            "unflagged GPS dropped"
        );
    }

    /// An `#exif` blob removed because no codec could read it falls back to
    /// stripping the metadata, never to shipping it.
    #[tokio::test]
    async fn a_removed_exif_blob_strips_the_metadata() {
        let mut doc = decode(PNG, Disposition::Redact, fixtures::png_with_gps()).await;
        doc.remove_part(&LocalId::new(EXIF_PART_ID)).unwrap();
        let out = doc.encode().unwrap();
        assert!(
            !fixtures::has_gps_png(out.as_bytes()),
            "GPS survived the removal"
        );
    }
}
