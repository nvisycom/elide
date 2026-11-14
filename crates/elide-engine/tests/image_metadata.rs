//! End-to-end: a JPEG's EXIF metadata is detected and redacted through the real
//! [`Orchestrator`], routed as the image's `#exif` metadata stream.
//!
//! Under a hidden-content policy that treats metadata as content, the image is
//! a depth-1 document and its `#exif` stream a depth-2 part. Every orchestrator
//! detects the labelled fields the scope's catalog declares (or every labelled
//! field, under `FlagEverything`) with no configuration, erases them by
//! default, and encodes the image once.

#![cfg(feature = "image")]

use bytes::Bytes;
use elide_core::entity::audit::{AuditEvent, ModelId};
use elide_core::entity::{Entity, LabelCatalog, builtins};
use elide_core::modality::metadata::Metadata;
use elide_core::primitive::Confidence;
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_core::redaction::{Disposition, HiddenContentPolicy};
use elide_core::{Component, Result};
use elide_detection::Analyzer;
use elide_engine::{Document, Orchestrator};
use elide_format::FormatRegistry;
use elide_image::fixtures::{has_gps, is_valid_image, jpeg_with_tags, pixel, text_tag};
use elide_image::modality::{Image, ImageLocation};
use elide_image::primitive::{BoundingBox, Dimensions, Point};
use elide_redaction::operators::{Erase, Keep, Replace};
use elide_redaction::{Anonymizer, Rule};

/// A small JPEG carrying a GPS latitude and an artist.
fn jpeg_with_exif() -> Bytes {
    jpeg_with_tags(true, &[("Artist", "Jane Roe")])
}

/// The artist `jpeg` carries, if any.
fn artist(jpeg: &[u8]) -> Option<String> {
    text_tag(jpeg, "Artist")
}

/// Decode `bytes` with metadata treated as content, run `orchestrator` over it,
/// and return the redacted bytes.
async fn redact(orchestrator: &Orchestrator, bytes: Bytes) -> Bytes {
    redact_under(orchestrator, bytes, Disposition::ScanAsContent).await
}

/// Decode `bytes` under the metadata disposition `metadata`, run
/// `orchestrator` over it, and return the redacted bytes.
async fn redact_under(orchestrator: &Orchestrator, bytes: Bytes, metadata: Disposition) -> Bytes {
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
    assert!(is_valid_image(&out), "output not a valid image");
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
    let bytes = jpeg_with_tags(
        false,
        &[("Make", "Acme"), ("Model", "X1"), ("Artist", "Jane Roe")],
    );

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
        .decode(bytes, "jpg", policy)
        .await
        .expect("decode jpeg");
    let mut documents = [Document::new("photo.jpg", handle)];
    orchestrator
        .anonymize(&mut documents)
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();

    assert_eq!(text_tag(&out, "Make"), None);
    assert_eq!(text_tag(&out, "Model"), None);
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
#[derive(Debug, Component)]
#[component(name = "whole-frame", version = "1.0.0")]
struct WholeFrame;

#[async_trait::async_trait]
impl Recognizer<Image> for WholeFrame {
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
            ModelId::new("model"),
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
    let before = pixel(&original, 0, 0).expect("decode fixture");

    let orchestrator = orchestrator().with_modality::<Image>(
        Analyzer::new().with_recognizer(WholeFrame),
        Anonymizer::new().with(Rule::fallback(Erase)),
    );
    let out = redact(&orchestrator, original).await;

    // Metadata track: GPS is gone.
    assert!(!has_gps(&out), "GPS survived the both-track redaction");
    // Pixel track: the frame was erased (blacked out), so the top-left pixel is
    // no longer the fixture's red.
    let after = pixel(&out, 0, 0).expect("decode output");
    assert_ne!(before, after, "pixels were not redacted");
}
