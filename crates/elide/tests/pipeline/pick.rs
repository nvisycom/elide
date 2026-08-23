//! The operator *pick* recorded into each entity's audit trail.
//!
//! Redaction resolves which operator hides each entity (and why) and records
//! that decision as a [`Selection`] event on the entity's own audit trail,
//! alongside the [`Redaction`] that applies it — no parallel selection object.
//! A review layer reads the picks straight off the entities. This exercises
//! that over a real multi-part container: run [`anonymize`] and read the picks
//! back from the returned report's body entities.
//!
//! [`Selection`]: elide::entity::audit::AuditKind::Selection
//! [`Redaction`]: elide::entity::audit::AuditKind::Redaction
//! [`anonymize`]: elide::Orchestrator::anonymize

use elide::codec::{FormatRegistry, PartId};
use elide::detection::Analyzer;
use elide::entity::audit::AuditKind;
use elide::entity::builtins;
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::{Erase, Replace};
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Orchestrator, Report, Result};

const SAMPLE: &[u8] = include_bytes!("../testdata/sample.docx");

/// Build an orchestrator whose body rule set is deterministic enough to read
/// back from the picks: email is replaced, everything else erased.
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

/// `anonymize` records a reviewable pick on every body entity: each names one of
/// the configured operators, is followed by its redaction, and the whole trail
/// still verifies. The fixture's email addresses route to the replace rule.
#[tokio::test]
async fn anonymize_records_reviewable_body_picks() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    let mut doc = registry.decode(SAMPLE, "docx").await?;
    let report = orchestrator.analyze(&mut doc, &Directives::new()).await?;
    let mut report = orchestrator.anonymize_with(&mut doc, report).await?;

    let body = report
        .entities::<Text>()
        .expect("the docx body is text and its pipeline is registered");
    assert!(!body.is_empty(), "the body detected entities to redact");

    for entity in body.iter() {
        let picked = entity
            .audit
            .selection()
            .expect("every redacted entity records its pick");
        let applied = entity
            .audit
            .redaction()
            .expect("the pick is followed by a redaction");
        let picked = picked.operator.name.as_str();
        assert!(
            picked == "replace" || picked == "erase",
            "each pick is one of the configured operators, got {picked}",
        );
        // The pick is recorded before it is applied, and the operator actually
        // run is the one that was picked.
        let selection_at = entity
            .audit
            .position(|k| matches!(k, AuditKind::Selection(_)));
        let redaction_at = entity
            .audit
            .position(|k| matches!(k, AuditKind::Redaction(_)));
        assert!(
            selection_at < redaction_at,
            "the Selection precedes the Redaction",
        );
        assert_eq!(
            picked, applied.operator.name,
            "the redaction ran the picked operator"
        );
        assert!(entity.audit.verify().is_ok(), "the trail still verifies");
    }
    // The fixture's email addresses route to the replace rule.
    assert!(
        body.iter()
            .any(|e| e.audit.selection().map(|s| s.operator.name.as_str()) == Some("replace")),
        "the email rule fires on the fixture's addresses",
    );
    Ok(())
}

/// A part with no detected entities routes through its pipeline and records no
/// picks — nothing to redact, no error. Built from a rebuilt report so the part
/// is present regardless of what the mock image backend detected.
#[tokio::test]
async fn anonymize_over_an_empty_part_records_nothing() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    let mut doc = registry.decode(SAMPLE, "docx").await?;
    // A report carrying one image part with no detected entities — apply routes
    // through the image pipeline (Anonymizer::new(), no rules) and does nothing.
    let image = PartId::new("word/media/image1.png");
    let report = Report::new().insert_part::<Image>(image.clone(), Vec::new());

    let mut report = orchestrator.anonymize_with(&mut doc, report).await?;
    assert!(
        report.entities::<Text>().is_none(),
        "the report has no body"
    );
    let part = report
        .part_entities::<Image>(&image)
        .expect("the image part routes to the image pipeline");
    assert!(part.is_empty(), "no entities on the part → no picks");
    Ok(())
}
