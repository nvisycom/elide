//! End-to-end: a JPEG's EXIF metadata is detected and redacted through the real
//! [`Orchestrator`], routed as the image's `#exif` metadata stream.
//!
//! Under a hidden-content policy that treats metadata as content, the image is
//! a depth-1 document and its `#exif` stream a depth-2 part. Every orchestrator
//! detects the labelled fields the scope's catalog declares (or every labelled
//! field, under `FlagEverything`) with no configuration, erases them by
//! default, and encodes the image once.

#![cfg(feature = "image")]

use elide_core::Result;
use elide_core::entity::audit::{AuditEvent, ModelId};
use elide_core::entity::{Entity, LabelCatalog, builtins};
use elide_core::modality::metadata::Metadata;
use elide_core::primitive::{ComponentId, Confidence};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_core::redaction::{Disposition, HiddenContentPolicy};
use elide_detection::Analyzer;
use elide_engine::{Document, Orchestrator};
use elide_format::FormatRegistry;
use elide_image::modality::{Image, ImageLocation};
use elide_image::primitive::{BoundingBox, Dimensions, Point};
use elide_redaction::operators::{Erase, Keep, Replace};
use elide_redaction::{Anonymizer, Rule};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata as ExifMetadata;

/// A small JPEG carrying a GPS latitude and an artist.
fn jpeg_with_exif() -> bytes::Bytes {
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
    exif.set_tag(ExifTag::Artist("Jane Roe".to_owned()));
    exif.write_to_vec(&mut bytes, FileExtension::JPEG).unwrap();
    bytes::Bytes::from(bytes)
}

/// The EXIF tag `tag` in `jpeg`, if present.
fn tag(jpeg: &[u8], tag: &ExifTag) -> Option<ExifTag> {
    ExifMetadata::new_from_vec(&jpeg.to_vec(), FileExtension::JPEG)
        .ok()?
        .get_tag(tag)
        .next()
        .cloned()
}

/// Whether `jpeg` still carries a GPS latitude tag.
fn has_gps(jpeg: &[u8]) -> bool {
    tag(jpeg, &ExifTag::GPSLatitude(Vec::new())).is_some()
}

/// The artist `jpeg` carries, if any.
fn artist(jpeg: &[u8]) -> Option<String> {
    match tag(jpeg, &ExifTag::Artist(String::new()))? {
        ExifTag::Artist(value) => Some(value.trim_end_matches('\0').to_owned()),
        _ => None,
    }
}

/// Decode `bytes` with metadata treated as content, run `orchestrator` over it,
/// and return the redacted bytes.
async fn redact(orchestrator: &Orchestrator, bytes: bytes::Bytes) -> bytes::Bytes {
    redact_under(orchestrator, bytes, Disposition::ScanAsContent).await
}

/// Decode `bytes` under the metadata disposition `metadata`, run
/// `orchestrator` over it, and return the redacted bytes.
async fn redact_under(
    orchestrator: &Orchestrator,
    bytes: bytes::Bytes,
    metadata: Disposition,
) -> bytes::Bytes {
    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(metadata);
    let handle = registry
        .decode(bytes, "jpg", policy)
        .await
        .expect("decode jpeg");
    let mut documents = [Document::new("photo.jpg", handle)];
    orchestrator
        .anonymize(&mut documents)
        .await
        .expect("anonymize");
    documents[0].document.encode().expect("encode").to_bytes()
}

/// An orchestrator over the builtin catalog with nothing configured.
fn orchestrator() -> Orchestrator {
    Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(FormatRegistry::with_builtin())
}

#[tokio::test]
async fn labelled_fields_are_detected_and_erased_with_nothing_configured() {
    let original = jpeg_with_exif();
    assert!(has_gps(&original), "fixture should carry GPS");

    let orchestrator = orchestrator();
    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let handle = registry
        .decode(original, "jpg", policy)
        .await
        .expect("decode jpeg");
    let mut documents = [Document::new("photo.jpg", handle)];
    let analyzed = orchestrator.analyze(&mut documents).await.expect("analyze");

    // The `#exif` stream was reached: a depth-2 part exists.
    assert!(
        analyzed.report.part_ids().any(|(id, _)| id.depth() == 2),
        "the #exif stream should be analyzed; parts: {:?}",
        analyzed.report.part_ids().collect::<Vec<_>>()
    );

    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();
    assert!(!has_gps(&out), "GPS survived");
    assert_eq!(artist(&out), None, "artist survived");
    assert!(
        image::load_from_memory(&out).is_ok(),
        "output not a valid image"
    );
}

#[tokio::test]
async fn a_metadata_anonymizer_overwrites_what_it_can() {
    let orchestrator = orchestrator().with_anonymizer::<Metadata>(
        Anonymizer::new()
            .with(Rule::label(
                builtins::PERSON_NAME.to_ref(),
                Replace::new("[REDACTED]"),
            ))
            .with(Rule::label(
                builtins::GEOLOCATION_METADATA.to_ref(),
                Replace::new("[REDACTED]"),
            )),
    );
    let out = redact(&orchestrator, jpeg_with_exif()).await;
    assert_eq!(artist(&out).as_deref(), Some("[REDACTED]"));
    // A coordinate cannot hold text, so the overwrite removes it.
    assert!(!has_gps(&out), "GPS survived the overwrite");
}

#[tokio::test]
async fn the_catalog_decides_which_fields_are_redacted() {
    let catalog: LabelCatalog = [(*builtins::PERSON_NAME).clone()].into_iter().collect();
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(catalog))
        .with_registry(FormatRegistry::with_builtin());
    let out = redact(&orchestrator, jpeg_with_exif()).await;
    assert_eq!(artist(&out), None, "artist survived");
    assert!(has_gps(&out), "GPS was redacted outside the catalog");
}

#[tokio::test]
async fn flag_everything_redacts_every_field_without_a_catalog() {
    let orchestrator = Orchestrator::new().with_registry(FormatRegistry::with_builtin());
    let out = redact_under(&orchestrator, jpeg_with_exif(), Disposition::FlagEverything).await;
    assert_eq!(artist(&out), None, "artist survived");
    assert!(!has_gps(&out), "GPS survived");
}

#[tokio::test]
async fn a_metadata_anonymizer_treats_flagged_fields() {
    let orchestrator = Orchestrator::new()
        .with_anonymizer::<Metadata>(
            Anonymizer::new()
                .with(Rule::label(
                    builtins::PERSON_NAME.to_ref(),
                    Replace::new("[REDACTED]"),
                ))
                .with(Rule::label(builtins::GEOLOCATION_METADATA.to_ref(), Keep)),
        )
        .with_registry(FormatRegistry::with_builtin());
    let out = redact_under(&orchestrator, jpeg_with_exif(), Disposition::FlagEverything).await;
    assert_eq!(artist(&out).as_deref(), Some("[REDACTED]"));
    assert!(has_gps(&out), "GPS was redacted despite a rule keeping it");
}

/// A rule keeping one field of a group loses to another field's erasure: the
/// field goes with its group, and a field outside the group stays.
#[tokio::test]
async fn a_kept_field_goes_with_its_group() {
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30])))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let mut exif = ExifMetadata::new();
    exif.set_tag(ExifTag::Make("Acme".to_owned()));
    exif.set_tag(ExifTag::Model("X1".to_owned()));
    exif.set_tag(ExifTag::Artist("Jane Roe".to_owned()));
    exif.write_to_vec(&mut bytes, FileExtension::JPEG).unwrap();

    let registry = FormatRegistry::with_builtin();
    let orchestrator = Orchestrator::new()
        .with_anonymizer::<Metadata>(
            Anonymizer::new()
                .with(Rule::<Metadata>::predicate(
                    |cx| cx.entity.location.key.as_str() != "Make",
                    Keep,
                ))
                .with(Rule::fallback(Erase)),
        )
        .with_registry(registry.clone());
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
    let handle = registry
        .decode(bytes::Bytes::from(bytes), "jpg", policy)
        .await
        .expect("decode jpeg");
    let mut documents = [Document::new("photo.jpg", handle)];
    orchestrator
        .anonymize(&mut documents)
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();

    assert!(tag(&out, &ExifTag::Make(String::new())).is_none());
    assert!(tag(&out, &ExifTag::Model(String::new())).is_none());
    assert_eq!(artist(&out).as_deref(), Some("Jane Roe"));
}

/// A blank overwrite holds nothing, so the field goes and the encode check has
/// nothing to object to.
#[tokio::test]
async fn overwriting_a_field_with_nothing_removes_it() {
    let orchestrator = Orchestrator::new()
        .with_anonymizer::<Metadata>(Anonymizer::new().with(Rule::label(
            builtins::PERSON_NAME.to_ref(),
            Replace::new(""),
        )))
        .with_registry(FormatRegistry::with_builtin());
    let out = redact_under(&orchestrator, jpeg_with_exif(), Disposition::FlagEverything).await;
    assert_eq!(artist(&out), None);
}

#[tokio::test]
async fn a_flagged_field_no_rule_covers_is_erased() {
    let orchestrator = Orchestrator::new()
        .with_anonymizer::<Metadata>(Anonymizer::new().with(Rule::label(
            builtins::PERSON_NAME.to_ref(),
            Replace::new("[REDACTED]"),
        )))
        .with_registry(FormatRegistry::with_builtin());
    let out = redact_under(&orchestrator, jpeg_with_exif(), Disposition::FlagEverything).await;
    assert_eq!(artist(&out).as_deref(), Some("[REDACTED]"));
    assert!(!has_gps(&out), "GPS survived with no rule covering it");
}

#[tokio::test]
async fn registering_a_metadata_analyzer_keeps_the_flagged_fields() {
    let orchestrator = orchestrator().with_modality::<Metadata>(
        Analyzer::new(),
        Anonymizer::new().with(Rule::fallback(Erase)),
    );
    let out = redact(&orchestrator, jpeg_with_exif()).await;
    assert!(!has_gps(&out), "GPS survived");
}

/// An image recognizer that flags the whole frame, so the pixel track redacts
/// and the buffer goes dirty, exercising the both-dirty composition.
#[derive(Debug)]
struct WholeFrame;

#[async_trait::async_trait]
impl Recognizer<Image> for WholeFrame {
    fn id(&self) -> ComponentId {
        ComponentId::new("whole-frame", "1.0.0")
    }

    async fn recognize(
        &self,
        subject: &Subject<Image>,
        _ctx: &Context<'_>,
    ) -> Result<Vec<Entity<Image>>> {
        let dims = subject.data().dimensions();
        let bbox = BoundingBox::from_origin(
            Point::new(0.0, 0.0),
            Dimensions::new(f64::from(dims.width), f64::from(dims.height)),
        );
        let location = ImageLocation::new(bbox);
        let event = AuditEvent::model(
            "whole-frame",
            Confidence::MAX,
            location.clone(),
            ModelId::default(),
        );
        let entity = Entity::new(builtins::PERSON_NAME.to_ref(), location).with_event(event);
        Ok(vec![entity])
    }
}

/// Both tracks at once: a JPEG whose PIXELS are redacted (image pipeline) AND
/// whose EXIF GPS is erased (built-in metadata pipeline) must compose into one
/// output carrying both edits, through the real analyze + anonymize.
#[tokio::test]
async fn pixel_redaction_and_gps_erasure_compose_end_to_end() {
    let original = jpeg_with_exif();
    let before = image::load_from_memory(&original).expect("decode fixture");

    let orchestrator = orchestrator().with_modality::<Image>(
        Analyzer::new().with_recognizer(WholeFrame),
        Anonymizer::new().with(Rule::fallback(Erase)),
    );
    let out = redact(&orchestrator, original).await;

    // Metadata track: GPS is gone.
    assert!(!has_gps(&out), "GPS survived the both-track redaction");
    // Pixel track: the frame was erased (blacked out), so the top-left pixel is
    // no longer the fixture's red.
    let after = image::load_from_memory(&out).expect("decode output");
    use image::GenericImageView;
    let (before_px, after_px) = (before.get_pixel(0, 0), after.get_pixel(0, 0));
    assert_ne!(before_px, after_px, "pixels were not redacted");
}
