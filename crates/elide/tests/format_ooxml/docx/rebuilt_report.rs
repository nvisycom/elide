//! The cross-process path: a [`Report`] rebuilt from scratch — as a consumer
//! would after serializing it, editing it elsewhere, and reconstructing it —
//! redacts the same as a freshly-analyzed one. The rebuilt report carries no
//! cached part handles, so `anonymize_with` re-decodes each part from the
//! container.
#![cfg(feature = "serde")]

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::audit::AuditKind;
use elide::entity::builtins;
use elide::modality::text::Text;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::{Erase, Replace};
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Orchestrator, Report, Result};

use super::{BODY_PART, FIXTURE};

#[tokio::test]
async fn rebuilt_report_redacts_via_redecode() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;
    let anonymizer = Anonymizer::new()
        .with(Rule::label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[EMAIL]"),
        ))
        .with(Rule::fallback(Erase));
    let orchestrator = Orchestrator::new()
        .with_registry(registry.clone())
        .with_modality::<Text>(Analyzer::new().with_recognizer(patterns), anonymizer);

    // Phase 1: analyze, then copy the body entities out — exactly what a caller
    // can serialize and ship to another process.
    let mut doc = registry.decode(FIXTURE.source, "docx").await?;
    let report = orchestrator.analyze(&mut doc, &Directives::new()).await?;
    let body = report
        .entities::<Text>()
        .map(|v| v.to_vec())
        .unwrap_or_default();
    assert!(!body.is_empty(), "the body should detect entities");

    // Phase 2: rebuild a FRESH report from the copied entities (no cached
    // handles), on a FRESH document handle, and apply. This forces the
    // re-decode path — the proof a deserialized report still redacts.
    let rebuilt = Report::new().insert_body::<Text>(body);
    let mut doc2 = registry.decode(FIXTURE.source, "docx").await?;
    let applied = orchestrator.anonymize_with(&mut doc2, rebuilt).await?;

    // Scan the *decompressed* body part, not the deflated archive bytes: a
    // substring check on the container could pass while the email survives.
    let encoded = doc2.encode()?;
    let body_part = elide_office::opc::test_util::read_part(encoded.as_bytes(), BODY_PART)
        .expect("body part present");
    let body_part = String::from_utf8(body_part).expect("body XML is UTF-8");
    assert!(
        !body_part.contains("bob.smith@example.com"),
        "a rebuilt report must still redact the body",
    );

    // The returned report carries the audit even on the re-decode path (this
    // report had no cached handles): the applied body entities each end with
    // a redaction event.
    let audited = applied
        .entities::<Text>()
        .map(|v| v.to_vec())
        .unwrap_or_default();
    assert!(
        !audited.is_empty(),
        "the applied body should surface entities"
    );
    for entity in &audited {
        assert!(
            matches!(
                entity.audit.events().last().map(|e| &e.kind),
                Some(AuditKind::Redaction(_))
            ),
            "each applied entity's final provenance event is its redaction",
        );
    }
    Ok(())
}
