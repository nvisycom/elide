//! The re-run path: `analyze` produces an enrichment artifact (OCR `Layout`)
//! alongside the report; that artifact is persisted and handed to `re_analyze`,
//! which re-recognizes over it *without* re-invoking the enricher. Proven by
//! re-running against an orchestrator whose OCR backend is empty: detection
//! survives only because the seeded artifact carried the OCR text through.

#![cfg(all(feature = "engine", feature = "ocr"))]

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::enrichment::ocr::{MockBackend, OcrEnricher};
use elide::entity::{Entity, LabelCatalog};
use elide::modality::image::{Image, ImageLocation, LayoutBlock};
use elide::primitive::{BoundingBox, Point};
use elide::recognition::Scope;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Orchestrator, Result};

/// Any decodable PNG: the mock OCR backend ignores the pixels and returns its
/// canned blocks, so the fixture only has to decode to an image.
const SAMPLE: &[u8] = include_bytes!("../testdata/sample.png");

fn loc() -> ImageLocation {
    ImageLocation::new(BoundingBox::from_origin_size(
        Point::new(0.0, 0.0),
        200.0,
        20.0,
    ))
}

/// An orchestrator whose image pipeline enriches with `backend` and detects
/// email addresses in the OCR text. The anonymizer erases what it finds.
fn orchestrator(registry: FormatRegistry, backend: MockBackend) -> Result<Orchestrator> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .build()?;
    let analyzer = Analyzer::new()
        .with_enricher(
            OcrEnricher::builder()
                .with_name("mock-ocr")
                .with_backend(backend)
                .build()?,
        )
        .with_recognizer(patterns);
    Ok(Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry)
        .with_modality::<Image>(analyzer, Anonymizer::new().with(Rule::fallback(Erase))))
}

fn image_entities(analyzed: &elide::AnalyzedDocument) -> Vec<Entity<Image>> {
    analyzed
        .report
        .entities::<Image>()
        .map(<[_]>::to_vec)
        .unwrap_or_default()
}

/// `re_analyze` reuses the prior OCR `Layout` instead of re-enriching: the
/// re-run finds the same entity even though its orchestrator's OCR backend is
/// empty, because the seeded artifact — not a fresh OCR call — supplies the text.
#[tokio::test]
async fn re_analyze_reuses_the_prior_ocr_artifact() -> Result<()> {
    // OCR text carrying a detectable email.
    let block = LayoutBlock::new(loc(), "write to alice@example.com today");

    // First pass: a real OCR backend produces the Layout, the pattern
    // recognizer detects the email over it.
    let first = orchestrator(
        FormatRegistry::with_builtin(),
        MockBackend::with(vec![block]),
    )?;
    let mut document = FormatRegistry::with_builtin().decode(SAMPLE, "png").await?;
    let analyzed = first.analyze(&mut document, &Directives::new()).await?;

    let found = image_entities(&analyzed);
    assert_eq!(found.len(), 1, "the email is detected in the OCR text");
    assert!(
        analyzed.artifacts.body::<Image>().is_some(),
        "analyze surfaces the OCR Layout as the body artifact",
    );

    // Second pass: re-run against an orchestrator whose OCR backend is EMPTY.
    // If re_analyze re-enriched, it would OCR nothing and detect nothing; it
    // instead seeds the recognition with the persisted Layout.
    let second = orchestrator(FormatRegistry::with_builtin(), MockBackend::new())?;
    let mut document = FormatRegistry::with_builtin().decode(SAMPLE, "png").await?;
    let reanalyzed = second
        .re_analyze(&mut document, &analyzed.artifacts, &Directives::new())
        .await?;

    let again = image_entities(&reanalyzed);
    assert_eq!(
        again.len(),
        1,
        "re_analyze detects the same email — the seeded Layout was reused, not re-OCR'd",
    );
    assert_eq!(
        again[0].label, found[0].label,
        "the re-run finds the same label as the first pass",
    );
    Ok(())
}

/// A control: without the seed, an empty OCR backend genuinely finds nothing —
/// so the reuse in the test above is what carries detection, not a stray match.
#[tokio::test]
async fn an_empty_backend_without_a_seed_finds_nothing() -> Result<()> {
    let orchestrator = orchestrator(FormatRegistry::with_builtin(), MockBackend::new())?;
    let mut document = FormatRegistry::with_builtin().decode(SAMPLE, "png").await?;
    let analyzed = orchestrator
        .analyze(&mut document, &Directives::new())
        .await?;
    assert!(
        image_entities(&analyzed).is_empty(),
        "an empty OCR backend yields no text, so nothing is detected",
    );
    Ok(())
}
