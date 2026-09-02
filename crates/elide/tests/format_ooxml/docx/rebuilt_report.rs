//! The cross-process path: a [`Report`] rebuilt from scratch — as a consumer
//! would after serializing it, editing it elsewhere, and reconstructing it —
//! redacts the same as a freshly-analyzed one. The rebuilt report carries no
//! cached part handles, so `anonymize_with` re-decodes each part from the
//! container.
#![cfg(feature = "serde")]

use elide::codec::FormatRegistry;
use elide::entity::audit::AuditKind;
use elide::modality::text::Text;
use elide::{Directives, PartId, RegistryDocumentExt, Report, Result};

use super::{BODY_PART, FIXTURE};
use crate::support::orchestrator::{TestOrchestrator, build_anonymizer, default_text_analyzer};

/// The fixture document's name — its depth-1 part key in the report.
const DOC: &str = "report.docx";

#[tokio::test]
async fn rebuilt_report_redacts_via_redecode() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = TestOrchestrator::bare()
        .with_registry(registry.clone())
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build();

    // Phase 1: analyze, then copy the document's own entities out — exactly what
    // a caller can serialize and ship to another process.
    let mut doc = registry.document(DOC, FIXTURE.source).await?;
    let report = orchestrator
        .analyze(&mut doc, &Directives::new())
        .await?
        .report;
    let body = report
        .entities::<Text>()
        .map(|v| v.to_vec())
        .unwrap_or_default();
    assert!(!body.is_empty(), "the document should detect entities");

    // Phase 2: rebuild a FRESH report from the copied entities (no cached
    // handles), on a FRESH document handle, and apply. This forces the
    // re-decode path — the proof a deserialized report still redacts.
    let rebuilt = Report::new().insert_part::<Text>(PartId::from(DOC), body);
    let mut doc2 = registry.document(DOC, FIXTURE.source).await?;
    let applied = orchestrator.anonymize_with(&mut doc2, rebuilt).await?;

    // Scan the *decompressed* body part, not the deflated archive bytes: a
    // substring check on the container could pass while the email survives.
    let encoded = doc2.handle.encode()?;
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
        "the applied document should surface entities"
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
