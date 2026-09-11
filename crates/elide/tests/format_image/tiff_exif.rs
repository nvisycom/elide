//! End-to-end through the **public facade API**: a TIFF's EXIF metadata is
//! detected and stripped alongside its pixels, driven by an `Orchestrator` a
//! caller assembles from the re-exported vocabulary.
//!
//! The TIFF counterpart of [`jpeg_exif`](super::jpeg_exif): TIFF stores EXIF as
//! the file's own IFD (not an embedded segment), so this exercises the
//! EXIF-as-IFD transfer path where the metadata is transplanted onto the
//! freshly-encoded container. The fixture and its assertions come from
//! `elide_image::test_util`, so this test needs no image/EXIF crate of its own.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::LabelCatalog;
use elide::modality::image::Image;
use elide::modality::metadata::Metadata;
use elide::recognition::Scope;
use elide::recognition::exif::ExifRecognizer;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Document, Orchestrator};
use elide_image::test_util::{has_gps_tiff, is_valid_image, tiff_with_gps};

#[tokio::test]
async fn tiff_gps_is_stripped_through_the_facade() {
    let original = tiff_with_gps();
    assert!(has_gps_tiff(&original), "fixture should carry GPS");

    let registry = FormatRegistry::with_builtin();
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        .with_modality::<Image>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        )
        .with_modality::<Metadata>(
            Analyzer::new().with_recognizer(ExifRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "tiff")
        .await
        .expect("decode tiff");
    let mut documents = [Document::new("photo.tiff", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
        .expect("analyze");
    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].handle.encode().expect("encode").to_bytes();
    assert!(
        !has_gps_tiff(&out),
        "GPS survived the facade end-to-end strip"
    );
    assert!(is_valid_image(&out), "output not a valid image");
}
