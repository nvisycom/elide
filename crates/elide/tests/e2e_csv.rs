//! End-to-end CSV codec round-trip: decode → analyze → anonymize → encode.
//!
//! The handler redacts PII per cell while the table structure (header row,
//! delimiters, non-sensitive cells) passes through unchanged.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use fixtures::asserts::{
    assert_label_present, assert_pii_removed, assert_preserved, assert_tokens_present,
};
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.csv"),
    source: include_bytes!("testdata/sample.csv"),
    extension: "csv",
};

#[tokio::test]
async fn csv_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run_tabular().await?;

    // Every sensitive column is detected across both data rows.
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

    // Both rows' sensitive values are gone from the re-encoded CSV.
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

    // CSV structure survives: header row and non-sensitive name cells stay.
    assert_preserved(
        &outcome.redacted_text(),
        &[
            "name,email,phone,card,iban,ssn,host",
            "Alice Johnson,",
            "Bob Smith,",
        ],
    );
    Ok(())
}
