//! End-to-end: a JPEG's EXIF metadata is detected and stripped through the real
//! [`Orchestrator`], routed as the image handler's `#exif` sub-part.
//!
//! Confirms the nested-metadata model composes through the engine's own
//! analyze + anonymize + bottom-up fold, with no engine changes: the image is a
//! depth-1 document, its `#exif` a depth-2 nested part; the metadata pipeline
//! detects the GPS field, `Erase` picks it for removal, and the fold re-encodes
//! the image once with the field gone.

#![cfg(all(feature = "image", feature = "metadata"))]

use elide_codec::FormatRegistry;
use elide_core::entity::LabelCatalog;
use elide_core::modality::image::Image;
use elide_core::modality::metadata::Metadata;
use elide_core::recognition::Scope;
use elide_detection::Analyzer;
use elide_engine::{Directives, Document, Orchestrator};
use elide_image::ExifRecognizer;
use elide_operator::operators::Erase;
use elide_redaction::{Anonymizer, Rule};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata as ExifMetadata;

/// A small JPEG carrying a GPS latitude tag.
fn jpeg_with_gps() -> bytes::Bytes {
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30])))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let mut exif = ExifMetadata::new();
    exif.set_tag(ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
        nominator: 51,
        denominator: 1,
    }]));
    exif.write_to_vec(&mut bytes, FileExtension::JPEG).unwrap();
    bytes::Bytes::from(bytes)
}

/// Whether `jpeg` bytes still carry a GPS latitude tag.
fn has_gps(jpeg: &[u8]) -> bool {
    ExifMetadata::new_from_vec(&jpeg.to_vec(), FileExtension::JPEG)
        .map(|m| {
            m.get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_some()
        })
        .unwrap_or(false)
}

#[tokio::test]
async fn jpeg_gps_is_detected_and_stripped_through_the_orchestrator() {
    let original = jpeg_with_gps();
    assert!(has_gps(&original), "fixture should carry GPS");

    let registry = FormatRegistry::with_builtin();
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        // Image pixels: no recognizer (nothing to detect in the frame here).
        .with_modality::<Image>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        )
        // Image EXIF: the ExifRecognizer surfaces the GPS field.
        .with_modality::<Metadata>(
            Analyzer::new().with_recognizer(ExifRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "jpg")
        .await
        .expect("decode jpeg");
    let mut documents = [Document::new("photo.jpg", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
        .expect("analyze");

    // The `#exif` sub-part was reached: a depth-2 Metadata part exists.
    let metadata_parts: Vec<_> = analyzed
        .report
        .part_ids()
        .filter(|(id, _)| id.depth() == 2)
        .collect();
    assert!(
        !metadata_parts.is_empty(),
        "the #exif sub-part should be analyzed; parts: {:?}",
        analyzed.report.part_ids().collect::<Vec<_>>()
    );

    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].handle.encode().expect("encode").to_bytes();
    assert!(!has_gps(&out), "GPS survived the end-to-end strip");
    // The output is still a valid decodable JPEG.
    assert!(
        image::load_from_memory(&out).is_ok(),
        "output not a valid image"
    );
}

/// An image recognizer that flags the whole frame, so the pixel track redacts
/// and the buffer goes dirty, exercising the both-dirty composition.
#[derive(Debug)]
struct WholeFrame;

#[async_trait::async_trait]
impl elide_core::recognition::Recognizer<Image> for WholeFrame {
    fn id(&self) -> elide_core::recognition::RecognizerId {
        elide_core::recognition::RecognizerId::new("whole-frame", "1.0.0")
    }

    async fn recognize(
        &self,
        data: &elide_core::modality::image::ImageData,
        _ctx: &elide_core::recognition::RecognizerContext<'_, Image>,
    ) -> elide_core::Result<elide_core::recognition::Recognition<Image>> {
        use elide_core::entity::audit::{AuditEvent, ModelEvent};
        use elide_core::entity::{Entity, builtins};
        use elide_core::modality::image::ImageLocation;
        use elide_core::primitive::{BoundingBox, Confidence, Point};

        let dims = &data.dimensions;
        let bbox = BoundingBox::from_origin_size(
            Point::new(0.0, 0.0),
            f64::from(dims.width),
            f64::from(dims.height),
        );
        let location = ImageLocation::new(bbox);
        let event = AuditEvent::model(
            "whole-frame",
            Confidence::MAX,
            location.clone(),
            ModelEvent::default(),
        );
        let entity = Entity::builder()
            .with_label(builtins::PERSON_NAME.to_ref())
            .with_location(location)
            .with_confidence(Confidence::MAX)
            .with_event(event)
            .build();
        Ok(elide_core::recognition::Recognition::new(
            entity.into_iter().collect(),
        ))
    }
}

/// Both tracks at once: a JPEG whose PIXELS are redacted (image pipeline) AND
/// whose EXIF GPS is stripped (metadata pipeline) must compose into one output
/// carrying both edits, through the real analyze + anonymize + fold.
#[tokio::test]
async fn pixel_redaction_and_gps_strip_compose_end_to_end() {
    let original = jpeg_with_gps();
    let before = image::load_from_memory(&original).expect("decode fixture");

    let registry = FormatRegistry::with_builtin();
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        .with_modality::<Image>(
            Analyzer::new().with_recognizer(WholeFrame),
            Anonymizer::new().with(Rule::fallback(Erase)),
        )
        .with_modality::<Metadata>(
            Analyzer::new().with_recognizer(ExifRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "jpg")
        .await
        .expect("decode jpeg");
    let mut documents = [Document::new("photo.jpg", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
        .expect("analyze");
    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].handle.encode().expect("encode").to_bytes();
    // Metadata track: GPS is gone.
    assert!(!has_gps(&out), "GPS survived the both-track strip");
    // Pixel track: the frame was erased (blacked out), so the top-left pixel is
    // no longer the fixture's red.
    let after = image::load_from_memory(&out).expect("decode output");
    use image::GenericImageView;
    let (before_px, after_px) = (before.get_pixel(0, 0), after.get_pixel(0, 0));
    assert_ne!(before_px, after_px, "pixels were not redacted");
}
