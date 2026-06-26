//! End-to-end DOCX codec round-trip: decode → analyze → anonymize → encode.
//!
//! A container format: the body XML is redacted as text and the embedded
//! image is driven by the orchestrator's image pipeline (mock LLM, detects
//! nothing), while the rest of the package (relationships, content-types)
//! passes through unchanged.

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

    // The embedded image survives as a valid PNG, and the structural parts
    // are still present.
    let image = outcome.part(IMAGE_PART).expect("image part present");
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n", "image part is not a PNG");
    assert!(
        outcome.part("[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    assert!(
        outcome.part("word/_rels/document.xml.rels").is_some(),
        "relationships part must survive",
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
    use elide::modality::image::Image;
    use elide::modality::text::Text;
    use elide::recognition::Scope;
    use elide::recognition::llm::LlmRecognizer;
    use elide::recognition::pattern::PatternRecognizer;
    use elide::redaction::Anonymizer;
    use elide::redaction::operators::{Erase, Replace};
    use elide::{Orchestrator, Report};

    let registry = FormatRegistry::with_builtin();
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;
    let anonymizer = Anonymizer::new()
        .with_label(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]"))
        .with_fallback(Erase);
    let orchestrator = Orchestrator::new(&registry)
        .with_modality::<Text>(
            Analyzer::new().with_recognizer(patterns),
            anonymizer,
            Scope::new(),
        )
        .with_modality::<Image>(
            Analyzer::new().with_recognizer(
                LlmRecognizer::<Image>::builder()
                    .with_name("mock-image")
                    .with_mock_backend()
                    .with_default_prompt()
                    .build()?,
            ),
            Anonymizer::new(),
            Scope::new(),
        );

    // Phase 1: analyze, then copy the entities out by modality — exactly what
    // a caller can serialize and ship to another process.
    let mut doc = registry.decode(FIXTURE.source, "docx").await?;
    let mut report = orchestrator.analyze(&mut doc).await?;
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
    orchestrator.anonymize_with(&mut doc2, rebuilt).await?;

    let encoded = doc2.encode()?;
    let redacted = String::from_utf8_lossy(encoded.as_bytes()).into_owned();
    assert!(
        !redacted.contains("alice.johnson@example.com"),
        "a rebuilt report must still redact the body",
    );
    Ok(())
}
