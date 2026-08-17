//! End-to-end DOCX codec round-trip: decode → analyze → anonymize → encode.
//!
//! The fixture is a real Word-authored document: its PII spans the body
//! (`word/document.xml`), a page header (`word/header3.xml`), and an external
//! hyperlink `mailto:` target in `word/_rels/document.xml.rels`, so redaction
//! must reach every text-bearing part and the relationship targets — not just
//! the body — while the styles, theme, and content-types parts pass through
//! unchanged.

mod fixtures;

use std::io::Read;

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
const HEADER_PART: &str = "word/header3.xml";
const RELS_PART: &str = "word/_rels/document.xml.rels";

/// Every PII value in the document, across its body, header, and relationships.
const PII: &[&str] = &[
    "alice.johnson@example.com",
    "bob.smith@example.com",
    "+1 (415) 555-0142",
    "+1 (510) 555-0199",
    "4111 1111 1111 1111",
    "GB29 NWBK 6016 1331 9268 19",
    "123-45-6789",
    "192.168.1.42",
];

#[tokio::test]
async fn docx_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The shipped patterns find every sensitive label across the document.
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

    // The body part: originals gone, replacement tokens in.
    let body = outcome.part(BODY_PART).expect("body part present");
    let body = String::from_utf8(body).expect("body XML is UTF-8");
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

    // The header carries an email too, and it is redacted in its own part.
    let header = outcome.part(HEADER_PART).expect("header part present");
    let header = String::from_utf8(header).expect("header XML is UTF-8");
    assert_pii_removed(&header, &["alice.johnson@example.com"]);
    assert_tokens_present(&header, &["[email_address]"]);

    // The body email is also an external hyperlink `mailto:` target in the
    // relationships part; redaction reaches it there too.
    let rels = outcome
        .part(RELS_PART)
        .expect("relationships part must survive");
    let rels = String::from_utf8(rels).expect("relationships XML is UTF-8");
    assert_pii_removed(&rels, &["bob.smith@example.com"]);
    assert_tokens_present(&rels, &["[email_address]"]);

    // The real guarantee: no PII value survives in ANY part of the output.
    for part in part_names(&outcome.redacted) {
        let bytes = outcome.part(&part).expect("listed part is readable");
        let text = String::from_utf8_lossy(&bytes);
        for pii in PII {
            assert!(!text.contains(pii), "PII `{pii}` survived in part `{part}`");
        }
    }

    // The content-types manifest still round-trips.
    assert!(
        outcome.part("[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    Ok(())
}

/// Every entry name in the redacted zip, so the leak scan covers all parts.
fn part_names(zip_bytes: &[u8]) -> Vec<String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).expect("output is a zip");
    (0..archive.len())
        .filter_map(|i| {
            let mut entry = archive.by_index(i).ok()?;
            // Skip binary parts (fonts, images) — the PII scan is over text.
            let name = entry.name().to_owned();
            if name.ends_with(".xml") || name.ends_with(".rels") {
                let mut buf = String::new();
                let _ = entry.read_to_string(&mut buf);
                Some(name)
            } else {
                None
            }
        })
        .collect()
}

/// A [`Report`] rebuilt from scratch — as a consumer would after serializing
/// it, editing it elsewhere, and reconstructing it — redacts the same as a
/// freshly-analyzed one. This is the cross-process path: the rebuilt report
/// carries no cached part handles, so `anonymize_with` re-decodes each part
/// from the container.
#[cfg(feature = "serde")]
#[tokio::test]
async fn rebuilt_report_redacts_via_redecode() -> Result<()> {
    use elide::codec::FormatRegistry;
    use elide::detection::Analyzer;
    use elide::entity::audit::AuditKind;
    use elide::modality::text::Text;
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
        .with_modality::<Text>(Analyzer::new().with_recognizer(patterns), anonymizer);

    // Phase 1: analyze, then copy the body entities out — exactly what a caller
    // can serialize and ship to another process.
    let mut doc = registry.decode(FIXTURE.source, "docx").await?;
    let mut report = orchestrator.analyze(&mut doc, &Directives::new()).await?;
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
    let mut applied = orchestrator.anonymize_with(&mut doc2, rebuilt).await?;

    let encoded = doc2.encode()?;
    let redacted = String::from_utf8_lossy(encoded.as_bytes()).into_owned();
    assert!(
        !redacted.contains("bob.smith@example.com"),
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
