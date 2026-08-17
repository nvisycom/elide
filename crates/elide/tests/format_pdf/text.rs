//! End-to-end born-digital PDF round-trip: decode → analyze → anonymize →
//! encode, entirely on the pure-Rust text path (no PDFium, no OCR).
//!
//! The fixture is a Courier text-layer PDF, so `elide-pdf` extracts its text
//! directly; the shipped patterns find the PII and the anonymizer rewrites it
//! in the re-encoded document.

use elide::Result;
use elide::codec::FormatRegistry;
use elide::entity::audit::AuditKind;
use elide::entity::builtins;
use elide::modality::StreamDataReader;
use elide::modality::text::Text;

use crate::support::asserts::{assert_label_present, assert_pii_removed};
use crate::support::pipeline::Fixture;

/// Re-decode a redacted PDF through the public registry and reassemble its
/// born-digital text. The re-encoded content stream is FlateDecode-compressed,
/// so the text is only legible through a real decode — not by grepping bytes.
async fn extracted_text(pdf: &[u8]) -> Result<String> {
    let registry = FormatRegistry::with_builtin();
    let mut handle = registry
        .decode(pdf, "pdf")
        .await?
        .into::<Text>()
        .expect("pdf is text");
    let mut text = String::new();
    while let Some(chunk) = handle.read_next().await? {
        text.push_str(chunk.data.as_str());
    }
    Ok(text)
}

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.pdf"),
    source: include_bytes!("../testdata/sample.pdf"),
    extension: "pdf",
};

/// The six PII values the `sample.pdf` fixture carries, none of which may
/// survive redaction.
const PII: [&str; 6] = [
    "alice.johnson@example.com",
    "555-0142",
    "4111 1111 1111 1111",
    "GB29 NWBK 6016 1331 9268 19",
    "123-45-6789",
    "192.168.1.42",
];

#[tokio::test]
async fn born_digital_pdf_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The born-digital text layer was extracted (not OCR'd), so the shipped
    // patterns find the PII spread the fixture carries.
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

    // Re-decode the redacted PDF: whichever path ran, none of the original PII
    // survives. PDF redaction removes the text (glyph deletion keeps a
    // selectable layer minus the detected spans; raster flattens to images), so
    // the guarantee asserted here is that the originals are gone — not that a
    // replacement token was inserted.
    let redacted = extracted_text(&outcome.redacted).await?;
    assert_pii_removed(&redacted, &PII);

    // Also assert the PII is absent from the raw output bytes. On the glyph
    // path the content stream is FlateDecode-compressed, so this is a weaker
    // check than the decoded one above — but it catches any PII that survives
    // uncompressed (annotations, metadata, an unfiltered stream).
    let raw = String::from_utf8_lossy(&outcome.redacted);
    for pii in PII {
        assert!(!raw.contains(pii), "PII survived in raw output: {pii}");
    }

    // The applied report carries the post-redaction audit.
    assert!(
        !outcome.audited.is_empty(),
        "the applied report should surface the redacted entities",
    );
    for entity in &outcome.audited {
        let last = entity
            .audit
            .events()
            .last()
            .expect("an applied entity has at least one event");
        assert!(
            matches!(last.kind, AuditKind::Redaction { .. }),
            "the final provenance event of a redacted entity is its redaction, got {:?}",
            last.kind,
        );
    }
    Ok(())
}
