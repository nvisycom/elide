//! A set of loose files redacted as one logical document: each file is a
//! top-level part keyed by its name, analyzed and applied on its own handle. A
//! set has no single body — every file, container or leaf, is a part. This
//! exercises the multi-document entry point and the `PartId` path model that
//! keeps two files' parts distinct.

#![cfg(all(
    feature = "engine",
    feature = "codec-png",
    feature = "codec-docx",
    feature = "test-utils",
))]

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::enrichment::ocr::{MockBackend, OcrEnricher};
use elide::entity::LabelCatalog;
use elide::modality::image::{Image, ImageLocation, LayoutBlock};
use elide::primitive::{BoundingBox, Point};
use elide::recognition::Scope;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Orchestrator, PartId, RegistryDocumentExt, Result};

/// A real (small) PNG the image codec decodes.
const SAMPLE_PNG: &[u8] = include_bytes!("../testdata/sample.png");

/// An orchestrator that redacts inside images: OCR text carrying a detectable
/// email, a pattern recognizer over it, an erase anonymizer.
fn orchestrator(registry: FormatRegistry) -> Result<Orchestrator> {
    let block = LayoutBlock::new(
        ImageLocation::new(BoundingBox::from_origin_size(
            Point::new(0.0, 0.0),
            200.0,
            20.0,
        )),
        "write to alice@example.com today",
    );
    let analyzer = Analyzer::new()
        .with_enricher(
            OcrEnricher::builder()
                .with_name("mock-ocr")
                .with_backend(MockBackend::with(vec![block]))
                .build()?,
        )
        .with_recognizer(
            PatternRecognizer::builder()
                .with_builtin_patterns()
                .build()?,
        );
    Ok(Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry)
        .with_modality::<Image>(analyzer, Anonymizer::new().with(Rule::fallback(Erase))))
}

/// A set of files with no wrapping archive is redacted as one logical document:
/// each file is a top-level part keyed by its name, analyzed and applied on its
/// own handle. Two plain image files (no container in sight) prove the
/// files-are-parts entry point — a set has *no* single body.
#[tokio::test]
async fn a_set_treats_each_file_as_a_named_part() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(registry.clone())?;

    // Two standalone images, supplied side by side.
    let mut documents = [
        registry.document("scan-A.png", SAMPLE_PNG).await?,
        registry.document("scan-B.png", SAMPLE_PNG).await?,
    ];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await?;

    // No single body: every file is a depth-1 part keyed by name, so a set of
    // two has two depth-1 parts (not one privileged body).
    let tops: Vec<_> = analyzed
        .report
        .part_ids()
        .filter(|(id, _)| id.depth() == 1)
        .map(|(id, _)| id.to_string())
        .collect();
    assert_eq!(
        tops.len(),
        2,
        "each file is its own depth-1 part; got: {tops:?}"
    );
    assert!(tops.contains(&"scan-A.png".to_string()), "got: {tops:?}");
    assert!(tops.contains(&"scan-B.png".to_string()), "got: {tops:?}");
    for name in ["scan-A.png", "scan-B.png"] {
        let id = PartId::new(name);
        assert!(
            analyzed.artifacts.part::<Image>(&id).is_some(),
            "the OCR layout for {name} was surfaced",
        );
    }

    // Applying redacts each file in place; each re-encodes on its own.
    let report = orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await?;
    assert!(!report.part_ids().collect::<Vec<_>>().is_empty());
    for file in &documents {
        assert!(
            !file.handle.encode()?.as_bytes().is_empty(),
            "each file re-encodes to non-empty bytes",
        );
    }
    Ok(())
}

/// A file in the set whose own BODY carries PII (a DOCX's `word/document.xml`)
/// is redacted, keyed by the file's name — proving each file's own content (not
/// only its embedded media) is analyzed, and that two files in one set are each
/// reached and redacted.
#[tokio::test]
async fn a_set_redacts_each_files_own_content() -> Result<()> {
    use elide::detection::Analyzer as TextAnalyzer;
    use elide::entity::builtins;
    use elide::modality::text::Text;
    use elide::recognition::pattern::PatternRecognizer as Patterns;
    use elide::redaction::operators::Replace;

    const SAMPLE_DOCX: &[u8] = include_bytes!("../testdata/sample.docx");

    let registry = FormatRegistry::with_builtin();
    let text = Anonymizer::new()
        .with(Rule::label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[EMAIL]"),
        ))
        .with(Rule::fallback(Erase));
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        .with_modality::<Text>(
            TextAnalyzer::new().with_recognizer(
                Patterns::builder()
                    .with_builtin_patterns()
                    .with_builtin_dictionaries()
                    .build()?,
            ),
            text,
        );

    // Two DOCX files supplied together; each carries an email in its body.
    let mut documents = [
        registry.document("first.docx", SAMPLE_DOCX).await?,
        registry.document("second.docx", SAMPLE_DOCX).await?,
    ];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await?;

    for name in ["first.docx", "second.docx"] {
        let id = PartId::new(name);
        let entities = analyzed
            .report
            .part_entities::<Text>(&id)
            .unwrap_or_else(|| panic!("{name}'s body reconstructs as Text"));
        assert!(
            !entities.is_empty(),
            "{name}'s body PII (an email) was detected — not leaked",
        );
    }

    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await?;
    for file in &documents {
        assert!(!file.handle.encode()?.as_bytes().is_empty());
    }
    Ok(())
}
