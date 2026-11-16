//! A reviewer's mark (a span named by its raw source bytes, with no decoded
//! span) is redacted like a detection; one the document cannot place fails
//! the apply rather than being skipped.

use std::ops::Range;

use elide::codec::FormatRegistry;
use elide::entity::audit::{Manual, ManualIntent};
use elide::entity::builtins;
use elide::modality::text::{SourceRef, Text, TextLocation};
use elide::redaction::HiddenContentPolicy;
use elide::{ErrorKind, Orchestrator, PartId, RegistryDocumentExt, Report, Result};

use crate::support::orchestrator::{TestOrchestrator, build_anonymizer, default_text_analyzer};
use crate::support::review::flagged;

const NAME: &str = "notes.json";
const SOURCE: &str = r#"{"note":"call 555-0100 today"}"#;

fn orchestrator(registry: &FormatRegistry) -> Result<Orchestrator> {
    Ok(TestOrchestrator::bare()
        .with_registry(registry.clone())
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build())
}

/// A report holding one reviewer mark over raw source bytes `range`.
fn marked(range: Range<usize>) -> Report {
    let mark = TextLocation::marked_source(SourceRef::new(range));
    Report::new().insert_part::<Text>(
        PartId::from(NAME),
        vec![flagged(builtins::PHONE_NUMBER.to_ref(), mark)],
    )
}

#[tokio::test]
async fn a_mark_is_redacted() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let mut document = registry
        .document(NAME, SOURCE.as_bytes(), HiddenContentPolicy::default())
        .await?;
    let at = SOURCE.find("555-0100").expect("in the source");
    let applied = orchestrator(&registry)?
        .anonymize_with(&mut document, marked(at..at + "555-0100".len()))
        .await?;

    let out = document.document.encode()?.decode()?;
    assert!(
        !out.contains("555-0100"),
        "the marked number survived: {out}"
    );
    assert!(out.contains("today"), "{out}");
    let entities = applied.part::<Text>(&document.id())?.entities();
    assert!(entities[0].is_redacted());
    assert!(entities[0].location.range().is_some(), "resolved to a span");
    Ok(())
}

#[tokio::test]
async fn a_mark_the_document_cannot_place_fails() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let mut document = registry
        .document(NAME, SOURCE.as_bytes(), HiddenContentPolicy::default())
        .await?;
    // `{` is structure, not a value.
    let Err(error) = orchestrator(&registry)?
        .anonymize_with(&mut document, marked(0..1))
        .await
    else {
        panic!("a mark on structure must fail");
    };
    assert_eq!(error.kind(), ErrorKind::MalformedInput);
    Ok(())
}

/// A suppressed mark is never redacted, so one the document cannot place
/// does not fail the apply.
#[tokio::test]
async fn a_suppressed_mark_is_not_resolved() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let mut document = registry
        .document(NAME, SOURCE.as_bytes(), HiddenContentPolicy::default())
        .await?;
    let mark = TextLocation::marked_source(SourceRef::new(0..1));
    let mut entity = flagged(builtins::PHONE_NUMBER.to_ref(), mark.clone());
    entity.record_manual(Manual::new(ManualIntent::Suppress, mark), Some("reviewer"));
    let report = Report::new().insert_part::<Text>(PartId::from(NAME), vec![entity]);
    orchestrator(&registry)?
        .anonymize_with(&mut document, report)
        .await?;
    assert_eq!(document.document.encode()?.decode()?, SOURCE);
    Ok(())
}
