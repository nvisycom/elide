//! End-to-end HTML codec round-trip: decode → analyze → anonymize → encode.
//!
//! The handler redacts PII in text nodes and attribute values while the tag
//! structure passes through unchanged.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use fixtures::asserts::{
    assert_label_present, assert_pii_removed, assert_preserved, assert_tokens_present,
};
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.html"),
    source: include_bytes!("testdata/sample.html"),
    extension: "html",
};

#[tokio::test]
async fn html_detects_and_redacts() -> Result<()> {
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

    // PII is gone from element text, including the values that also
    // appear in attributes and the comment.
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

    // Markup structure survives: tags and non-sensitive text stay.
    assert_preserved(
        &outcome.redacted_text(),
        &["<html", "<body>", "<h1>Customer onboarding</h1>", "Best,"],
    );
    Ok(())
}
