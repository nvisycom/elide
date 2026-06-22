//! End-to-end JSON codec round-trip: decode → analyze → anonymize → encode.
//!
//! The handler redacts PII in string values while the surrounding structure
//! (keys, braces, array shape) passes through unchanged.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use fixtures::asserts::{
    assert_label_present, assert_pii_removed, assert_preserved, assert_tokens_present,
};
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.json"),
    source: include_bytes!("testdata/sample.json"),
    extension: "json",
};

#[tokio::test]
async fn json_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

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

    // Both rows' sensitive values are gone.
    assert_pii_removed(
        &outcome.redacted_text(),
        &[
            "alice.johnson@example.com",
            "bob.smith@example.com",
            "+1 (415) 555-0142",
            "+1 (510) 555-0199",
            "4111 1111 1111 1111",
            "5555 5555 5555 4444",
            "GB29 NWBK 6016 1331 9268 19",
            "123-45-6789",
            "192.168.1.42",
        ],
    );

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

    // JSON structure survives: keys, braces, and the non-sensitive
    // subject all stay verbatim.
    assert_preserved(
        &outcome.redacted_text(),
        &[
            "\"subject\"",
            "Customer onboarding",
            "\"contacts\"",
            "\"email\"",
            "\"host\"",
        ],
    );
    assert!(
        outcome.redacted_text().trim_start().starts_with('{'),
        "redacted JSON lost its opening brace:\n{}",
        outcome.redacted_text(),
    );
    Ok(())
}
