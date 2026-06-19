//! End-to-end plain-text codec round-trip: decode → analyze → anonymize
//! → encode, asserting the shipped patterns detect the PII spread and the
//! anonymizer rewrites it while preserving the surrounding text.

mod fixtures;

use elide::entity::builtins;
use fixtures::asserts::{
    assert_label_present, assert_pii_removed, assert_preserved, assert_tokens_present,
};
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/contact.txt"),
    source: include_str!("testdata/contact.txt"),
    extension: "txt",
};

#[tokio::test]
async fn txt_detects_and_redacts() {
    let outcome = FIXTURE.run().await;

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
        &outcome.redacted,
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
        &outcome.redacted,
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
        &outcome.redacted,
        &["Subject: Customer onboarding", "Hi team,", "Best,"],
    );
}
