//! Per-country identifier columns: a spread of national IDs, tax IDs, and postal
//! codes across locales, each column headed by a keyword that supplies context.
//! Exercises the breadth of the locale-scoped detectors in a structured table.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/csv/locale_identifiers.csv"
    ),
    source: include_bytes!("../testdata/csv/locale_identifiers.csv"),
    extension: "csv",
};

#[tokio::test]
async fn locale_identifiers_are_detected() -> Result<()> {
    let outcome = FIXTURE.run_tabular().await?;

    // National IDs, tax IDs, and postal codes across locales are detected.
    assert_label_present!(
        outcome.entities,
        builtins::GOVERNMENT_ID.to_ref(),
        builtins::TAX_ID.to_ref(),
        builtins::POSTAL_CODE.to_ref(),
    );

    // The identifiers are removed.
    assert_pii_removed!(
        outcome.redacted_text(),
        "123-45-6789",
        "912-85-1234",
        "AB123456C",
        "12345678Z",
        "DE123456788",
        "ABCPE1234F",
    );

    // The country column and header survive.
    assert_content_preserved!(
        outcome.redacted_text(),
        "country,national_id,tax_id,postal_code",
        "US,",
        "UK,",
    );
    Ok(())
}
