//! The review round trip: analyze → serialize the report → ship it out for
//! editing → [`deserialize_report`] it back → [`anonymize_with`]. The
//! reconstructed report redacts exactly as the original would have.
//!
//! [`deserialize_report`]: elide::Orchestrator::deserialize_report
//! [`anonymize_with`]: elide::Orchestrator::anonymize_with

#![cfg(feature = "engine")]

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::builtins;
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::{Erase, Replace};
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Orchestrator, Result};

const SAMPLE: &[u8] = include_bytes!("../testdata/sample.docx");

fn orchestrator(registry: &FormatRegistry) -> Result<Orchestrator<'_>> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;
    let text = Anonymizer::new()
        .with(Rule::label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[EMAIL]"),
        ))
        .with(Rule::fallback(Erase));
    let image = LlmRecognizer::<Image>::builder()
        .with_name("mock-image")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;
    Ok(Orchestrator::new(registry)
        .with_modality::<Text>(Analyzer::new().with_recognizer(patterns), text)
        .with_modality::<Image>(Analyzer::new().with_recognizer(image), Anonymizer::new()))
}

/// A report survives a JSON round trip through the orchestrator: the same
/// entities come back, keyed to the same modality, and `anonymize_with` on the
/// rebuilt report redacts and stamps the full pick→redaction trail — exactly as
/// applying the original would have.
#[tokio::test]
async fn report_round_trips_through_serialize_and_deserialize() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    // Analyze, then serialize the report — the artifact a review layer ships.
    let mut doc = registry.decode(SAMPLE, "docx").await?;
    let mut report = orchestrator.analyze(&mut doc, &Directives::new()).await?;
    let detected = report.entities::<Text>().expect("text body").len();
    assert!(detected > 0, "the fixture detected body entities");
    let json = serde_json::to_string(&report).expect("report serializes");

    // Rebuild it from the wire through the same orchestrator.
    let mut de = serde_json::Deserializer::from_str(&json);
    let mut rebuilt = orchestrator.deserialize_report(&mut de)?;

    // Same entities, same modality, audit trail intact.
    let body = rebuilt
        .entities::<Text>()
        .expect("body reconstructed as Text");
    assert_eq!(body.len(), detected, "every detected entity survived");
    assert!(
        body.iter().all(|e| e.audit.verify().is_ok()),
        "each reconstructed entity's audit trail still verifies",
    );

    // The rebuilt report (no cached handles) redacts against a fresh decode of
    // the same document, stamping pick + redaction on every entity.
    let mut doc2 = registry.decode(SAMPLE, "docx").await?;
    let mut applied = orchestrator.anonymize_with(&mut doc2, rebuilt).await?;
    for entity in applied.entities::<Text>().expect("applied body").iter() {
        assert!(entity.audit.selection().is_some(), "records the pick");
        assert!(entity.is_redacted(), "records the redaction");
    }
    // The document re-encodes after redaction.
    assert!(!doc2.encode()?.as_bytes().is_empty());
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
    let orchestrator = Orchestrator::new(&registry);

    // A non-empty text group against a registry with no text pipeline.
    let json = r#"{"body":{"modality":"text","entities":[{}]},"parts":{}}"#;
    let mut de = serde_json::Deserializer::from_str(json);
    let err = match orchestrator.deserialize_report(&mut de) {
        Ok(_) => panic!("no text pipeline registered — should have failed"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), elide::ErrorKind::MalformedInput);
    Ok(())
}
