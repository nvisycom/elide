//! A reviewer's mark on a DOCX, named by raw bytes in a part's XML with no
//! decoded span, is redacted like a detection.

use elide::codec::FormatRegistry;
use elide::entity::builtins;
use elide::modality::text::{Text, TextLocation};
use elide::redaction::HiddenContentPolicy;
use elide::{PartId, RegistryDocumentExt, Report, Result};

use super::{BODY_PART, FIXTURE};
use crate::support::orchestrator::{TestOrchestrator, build_anonymizer, default_text_analyzer};
use crate::support::review::flagged;

const DOC: &str = "report.docx";
const EMAIL: &str = "bob.smith@example.com";

/// The body part's XML in `docx`.
fn body_xml(docx: &[u8]) -> String {
    let part = elide_office::fixtures::read_part(docx, BODY_PART).expect("body part present");
    String::from_utf8(part).expect("body XML is UTF-8")
}

#[tokio::test]
async fn a_mark_on_raw_part_bytes_is_redacted() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = TestOrchestrator::bare()
        .with_registry(registry.clone())
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build();

    // Source refs address a part as decoded (hidden content sanitized away),
    // so take the email's from a detection of it.
    let mut document = registry
        .document(DOC, FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let report = orchestrator.analyze(&mut document).await?.report;
    let refs = report
        .part::<Text>(&document.id())?
        .entities()
        .iter()
        .filter(|entity| entity.label == builtins::EMAIL_ADDRESS.to_ref())
        .map(|entity| entity.location.source_refs())
        .find(|refs| {
            refs.iter()
                .any(|source| source.part.as_deref() == Some(BODY_PART))
        })
        .expect("the body's email is detected")
        .to_vec();
    let mark = TextLocation::marked_sources(refs).expect("refs are non-empty");

    // Apply only the mark, on a fresh document.
    let report = Report::new().insert_part::<Text>(
        PartId::from(DOC),
        vec![flagged(builtins::EMAIL_ADDRESS.to_ref(), mark)],
    );
    let mut document = registry
        .document(DOC, FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let applied = orchestrator.anonymize_with(&mut document, report).await?;

    let body = body_xml(document.document.encode()?.as_bytes());
    assert!(!body.contains(EMAIL), "the marked email survived");
    let entities = applied.part::<Text>(&document.id())?.entities();
    assert!(entities[0].is_redacted());
    Ok(())
}
