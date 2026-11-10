//! End-to-end through the **public facade API**: a JPEG's EXIF metadata is
//! detected and stripped alongside its pixels, driven by an `Orchestrator` a
//! caller assembles from the re-exported vocabulary.
//!
//! Proves the built-in metadata pipeline is reachable and wires cleanly: the
//! image is a document, its EXIF the `#exif` stream; the labelled GPS field is
//! detected with nothing configured and erased in the one re-encode. The fixture and its assertions come from
//! `elide_image::fixtures`, so this test needs no image/EXIF crate of its own.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::{LabelCatalog, builtins};
use elide::modality::image::Image;
use elide::modality::metadata::Metadata;
use elide::recognition::Scope;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Disposition, HiddenContentPolicy, Rule};
use elide::{Directives, Document, Orchestrator};
use elide_image::fixtures::{
    description, has_gps, is_valid_image, jpeg_with_description, jpeg_with_gps,
};

use crate::support::orchestrator::{build_analyzer, build_anonymizer};

#[tokio::test]
async fn jpeg_gps_is_stripped_through_the_facade() {
    let original = jpeg_with_gps();
    assert!(has_gps(&original), "fixture should carry GPS");

    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        .with_modality::<Image>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "jpg", policy)
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

    let out = documents[0].document.encode().expect("encode").to_bytes();
    assert!(!has_gps(&out), "GPS survived the facade end-to-end strip");
    assert!(is_valid_image(&out), "output not a valid image");
}

/// A recognizer reads a free-text field's value: what it finds is redacted and
/// the rest of the field ships.
#[tokio::test]
async fn a_finding_inside_an_image_description_is_redacted_alone() {
    let original = jpeg_with_description("Taken by bob@example.com at the lake");
    let registry = FormatRegistry::with_builtin();
    let catalog = LabelCatalog::from_iter([builtins::EMAIL_ADDRESS.clone()]);
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(catalog))
        .with_registry(registry.clone())
        .with_modality::<Metadata>(
            build_analyzer::<Metadata>().expect("analyzer"),
            build_anonymizer::<Metadata>(),
        );
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let handle = registry
        .decode(original, "jpg", policy)
        .await
        .expect("decode jpeg");
    let mut documents = [Document::new("photo.jpg", handle)];
    orchestrator
        .anonymize(&mut documents, &Directives::new())
        .await
        .expect("anonymize");

    let out = documents[0].document.encode().expect("encode").to_bytes();
    assert_eq!(
        description(&out).as_deref(),
        Some("Taken by [email_address] at the lake")
    );
    assert!(is_valid_image(&out), "output not a valid image");
}
