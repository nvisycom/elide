//! The review round trip: analyze → serialize the report → ship it out for
//! editing → [`deserialize_report`] it back → [`anonymize_with`]. The
//! reconstructed report redacts exactly as the original would have.
//!
//! [`deserialize_report`]: elide::Orchestrator::deserialize_report
//! [`anonymize_with`]: elide::Orchestrator::anonymize_with

#![cfg(all(feature = "engine", feature = "test-utils", feature = "llm"))]

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::LabelCatalog;
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::recognition::Scope;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::Replace;
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Orchestrator, RegistryDocumentExt, Result};

use crate::support::orchestrator::TestOrchestrator;

const SAMPLE: &[u8] = include_bytes!("../testdata/sample.docx");

/// The fixture document's name — its depth-1 part key in the report.
const DOC: &str = "sample.docx";

fn orchestrator(registry: FormatRegistry) -> Result<Orchestrator> {
    Ok(TestOrchestrator::new()?.with_registry(registry).build())
}

/// A report survives a JSON round trip through the orchestrator: the same
/// entities come back, keyed to the same modality, and `anonymize_with` on the
/// rebuilt report redacts and stamps the full pick→redaction trail — exactly as
/// applying the original would have.
#[tokio::test]
async fn report_round_trips_through_serialize_and_deserialize() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(registry.clone())?;

    // Analyze, then serialize the report — the artifact a review layer ships.
    let mut doc = registry.document(DOC, SAMPLE).await?;
    let report = orchestrator
        .analyze(&mut doc, &Directives::new())
        .await?
        .report;
    let detected = report.entities::<Text>().expect("text content").len();
    assert!(detected > 0, "the fixture detected entities");
    let json = serde_json::to_string(&report).expect("report serializes");

    // Rebuild it from the wire through the same orchestrator.
    let mut de = serde_json::Deserializer::from_str(&json);
    let rebuilt = orchestrator.deserialize_report(&mut de)?;

    // Same entities, same modality, audit trail intact.
    let body = rebuilt
        .entities::<Text>()
        .expect("content reconstructed as Text");
    assert_eq!(body.len(), detected, "every detected entity survived");
    assert!(
        body.iter().all(|e| e.audit.verify().is_ok()),
        "each reconstructed entity's audit trail still verifies",
    );

    // The rebuilt report (no cached handles) redacts against a fresh decode of
    // the same document, stamping pick + redaction on every entity.
    let mut doc2 = registry.document(DOC, SAMPLE).await?;
    let applied = orchestrator.anonymize_with(&mut doc2, rebuilt).await?;
    for entity in applied.entities::<Text>().expect("applied content").iter() {
        assert!(entity.audit.selection().is_some(), "records the pick");
        assert!(entity.is_redacted(), "records the redaction");
    }
    // The document re-encodes after redaction.
    assert!(!doc2.handle.encode()?.as_bytes().is_empty());
    Ok(())
}

/// A report whose group carries entities under a modality the orchestrator does
/// not handle is rejected, rather than silently losing those (possibly
/// reviewer-edited) entities. An *empty* unregistered group would instead be
/// skipped — see the engine unit tests.
#[tokio::test]
async fn deserialize_rejects_an_unregistered_modality_with_entities() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    // An orchestrator with *no* modalities registered.
    let orchestrator = Orchestrator::new().with_registry(registry);

    // A non-empty text group against a registry with no text pipeline.
    let json = r#"{"parts":[{"id":["document"],"modality":"text","entities":[{}]}]}"#;
    let mut de = serde_json::Deserializer::from_str(json);
    let err = match orchestrator.deserialize_report(&mut de) {
        Ok(_) => panic!("no text pipeline registered — should have failed"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), elide::ErrorKind::MalformedInput);
    Ok(())
}

/// The standalone `Report::deserializer()` rebuilds a serialized report with no
/// orchestrator — the review-layer path that carries no analyzers, anonymizers,
/// or codec registry. The rebuilt report is byte-for-byte the same shape as one
/// deserialized through the orchestrator.
#[tokio::test]
async fn report_deserializer_rebuilds_a_report_standalone() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(registry.clone())?;

    let mut doc = registry.document(DOC, SAMPLE).await?;
    let report = orchestrator
        .analyze(&mut doc, &Directives::new())
        .await?
        .report;
    let detected = report.entities::<Text>().expect("text content").len();
    let json = serde_json::to_string(&report).expect("serializes");

    // No orchestrator here — just the modalities the report may contain.
    let mut de = serde_json::Deserializer::from_str(&json);
    let rebuilt = elide::Report::deserializer()
        .with_modality::<Text>()
        .with_modality::<Image>()
        .deserialize(&mut de)?;

    assert_eq!(
        rebuilt
            .entities::<Text>()
            .expect("content reconstructed")
            .len(),
        detected,
    );
    Ok(())
}

/// `with_analyzer` and `with_anonymizer` register each pipeline half separately,
/// merging into one pipeline — equivalent to `with_modality(analyzer,
/// anonymizer)`. Detection and redaction both run, so neither half was lost.
#[tokio::test]
async fn split_pipeline_halves_detect_and_redact() -> Result<()> {
    let registry = FormatRegistry::with_builtin();

    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;
    let text_redact = Anonymizer::new().with(Rule::fallback(Replace::default()));

    // Register the two halves in separate calls — no fabricated empty half.
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        .with_analyzer::<Text>(Analyzer::new().with_recognizer(patterns))
        .with_anonymizer::<Text>(text_redact);

    let mut doc = registry.document(DOC, SAMPLE).await?;
    let applied = orchestrator.anonymize(&mut doc, &Directives::new()).await?;

    let body = applied.entities::<Text>().expect("text content");
    assert!(!body.is_empty(), "the analyzer half detected entities");
    assert!(
        body.iter().all(|e| e.is_redacted()),
        "the anonymizer half redacted them — both halves survived the merge",
    );
    Ok(())
}
