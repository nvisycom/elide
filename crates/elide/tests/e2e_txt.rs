//! End-to-end TXT codec round-trip: decode → analyze → anonymize → encode.
//!
//! The shipped patterns detect the PII spread and the anonymizer rewrites
//! it, while the surrounding prose passes through unchanged.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use elide::entity::provenance::EventKind;
use fixtures::asserts::{
    assert_label_present, assert_pii_removed, assert_preserved, assert_tokens_present,
};
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.txt"),
    source: include_bytes!("testdata/sample.txt"),
    extension: "txt",
};

#[tokio::test]
async fn txt_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The shipped patterns find every sensitive value in the fixture.
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

    // Originals are gone from the re-encoded document.
    assert_pii_removed(
        &outcome.redacted_text(),
        &[
            "alice.johnson@example.com",
            "+1 (415) 555-0142",
            "4111 1111 1111 1111",
            "GB29 NWBK 6016 1331 9268 19",
            "123-45-6789",
            "192.168.1.42",
        ],
    );

    // Replacement tokens are present (payment card is masked, not tokened).
    assert_tokens_present(
        &outcome.redacted_text(),
        &[
            "[email_address]",
            "[phone_number]",
            "[iban]",
            "[government_id]",
            "[ip_address]",
        ],
    );

    // Non-sensitive prose passes through untouched.
    assert_preserved(
        &outcome.redacted_text(),
        &["Subject: Customer onboarding", "Hi team,", "Best,"],
    );

    // The report `anonymize_with` returns carries the post-redaction audit:
    // every applied entity gained a redaction event in its provenance.
    assert!(
        !outcome.audited.is_empty(),
        "the applied report should surface the redacted entities",
    );
    for entity in &outcome.audited {
        let last = entity
            .provenance
            .events
            .last()
            .expect("an applied entity has at least one event");
        assert!(
            matches!(last.kind, EventKind::Redaction { .. }),
            "the final provenance event of a redacted entity is its redaction, got {:?}",
            last.kind,
        );
    }
    Ok(())
}
