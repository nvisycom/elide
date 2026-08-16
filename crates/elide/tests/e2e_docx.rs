//! End-to-end DOCX codec round-trip: decode → analyze → anonymize → encode.
//!
//! A container format: the body XML is redacted as text and the embedded
//! image is driven by the orchestrator's image pipeline (mock LLM, detects
//! nothing), while the content-types manifest passes through unchanged.
//!
//! The fixture is shaped like a real generator (docx.js) output: its XML parts
//! carry a leading UTF-8 BOM, so detection must find the body PII against the
//! true byte offsets rather than shift off them; and its two emails live both
//! in the body and as external hyperlink `mailto:` targets in
//! `word/_rels/document.xml.rels`, so redaction must reach the targets in the
//! relationships part, not just the body.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use fixtures::asserts::{assert_label_present, assert_pii_removed, assert_tokens_present};
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.docx"),
    source: include_bytes!("testdata/sample.docx"),
    extension: "docx",
};

const BODY_PART: &str = "word/document.xml";
const IMAGE_PART: &str = "word/media/image1.png";
const RELS_PART: &str = "word/_rels/document.xml.rels";

#[tokio::test]
async fn docx_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The shipped patterns find the same labels they do in the other
    // `sample.*` fixtures.
    for label in [
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::PHONE_NUMBER.to_ref(),
        builtins::PAYMENT_CARD.to_ref(),
        builtins::IBAN.to_ref(),
        builtins::GOVERNMENT_ID.to_ref(),
        builtins::IP_ADDRESS.to_ref(),
    ] {
        assert_label_present(&outcome.entities, &label);
    }

    // The body XML part: originals gone, replacement tokens in.
    let body = outcome.part(BODY_PART).expect("body part present");
    let body = String::from_utf8(body).expect("body XML is UTF-8");
    assert_pii_removed(
        &body,
        &[
            "alice.johnson@example.com",
            "bob.smith@example.com",
            "+1 (415) 555-0142",
            "+1 (510) 555-0199",
            "4111 1111 1111 1111",
            "GB29 NWBK 6016 1331 9268 19",
            "123-45-6789",
            "192.168.1.42",
        ],
    );
    assert_tokens_present(
        &body,
        &[
            "[email_address]",
            "[phone_number]",
            "[iban]",
            "[government_id]",
            "[ip_address]",
        ],
    );

    // The two emails are also external hyperlink `mailto:` targets in the
    // relationships part. Redaction must reach them there — the part survives,
    // but the plaintext addresses are gone and the redaction token is in their
    // place.
    let rels = outcome
        .part(RELS_PART)
        .expect("relationships part must survive");
    let rels = String::from_utf8(rels).expect("relationships XML is UTF-8");
    assert_pii_removed(
        &rels,
        &["alice.johnson@example.com", "bob.smith@example.com"],
    );
    assert_tokens_present(&rels, &["[email_address]"]);

    // The embedded image survives as a valid PNG, and the content-types
    // manifest is still present.
    let image = outcome.part(IMAGE_PART).expect("image part present");
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n", "image part is not a PNG");
    assert!(
        outcome.part("[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    Ok(())
}

/// A [`Report`] rebuilt from scratch — as a consumer would after serializing
/// it, editing it elsewhere, and reconstructing it — redacts the same as a
/// freshly-analyzed one. This is the cross-process path: the rebuilt report
/// carries no cached part handles, so `anonymize_with` re-decodes each part
/// from the container.
#[cfg(feature = "serde")]
#[tokio::test]
async fn rebuilt_report_redacts_via_redecode() -> Result<()> {
    use elide::codec::{FormatRegistry, PartId};
    use elide::detection::Analyzer;
    use elide::entity::audit::AuditKind;
    use elide::modality::image::Image;
    use elide::modality::text::Text;
    use elide::recognition::llm::LlmRecognizer;
    use elide::recognition::pattern::PatternRecognizer;
    use elide::redaction::operators::{Erase, Replace};
    use elide::redaction::{Anonymizer, Rule};
    use elide::{Directives, Orchestrator, Report};

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
    let orchestrator = Orchestrator::new(&registry)
        .with_modality::<Text>(Analyzer::new().with_recognizer(patterns), anonymizer)
        .with_modality::<Image>(
            Analyzer::new().with_recognizer(
                LlmRecognizer::<Image>::builder()
                    .with_name("mock-image")
                    .with_mock_backend()
                    .with_default_prompt()
                    .build()?,
            ),
            Anonymizer::new(),
        );

    // Phase 1: analyze, then copy the entities out by modality — exactly what
    // a caller can serialize and ship to another process.
    let mut doc = registry.decode(FIXTURE.source, "docx").await?;
    let mut report = orchestrator.analyze(&mut doc, &Directives::new()).await?;
    let body = report
        .entities::<Text>()
        .map(|v| v.to_vec())
        .unwrap_or_default();
    let image_part = PartId::new(IMAGE_PART);
    let part = report
        .part_entities::<Image>(&image_part)
        .map(|v| v.to_vec())
        .unwrap_or_default();
    assert!(!body.is_empty(), "the body should detect entities");

    // Phase 2: rebuild a FRESH report from the copied entities (no cached
    // handles), on a FRESH document handle, and apply. This forces the
    // re-decode path — the proof a deserialized report still redacts.
    let rebuilt = Report::new()
        .insert_body::<Text>(body)
        .insert_part::<Image>(image_part, part);
    let mut doc2 = registry.decode(FIXTURE.source, "docx").await?;
    let mut applied = orchestrator.anonymize_with(&mut doc2, rebuilt).await?;

    let encoded = doc2.encode()?;
    let redacted = String::from_utf8_lossy(encoded.as_bytes()).into_owned();
    assert!(
        !redacted.contains("alice.johnson@example.com"),
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
                Some(AuditKind::Redaction { .. })
            ),
            "each applied entity's final provenance event is its redaction",
        );
    }
    Ok(())
}
