//! PII inside quoted cells that carry embedded commas, embedded newlines, and
//! doubled-quote (`""`) escapes: the loader must parse these as single cells,
//! redact the PII in them, and re-serialize the quoting faithfully.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/csv/quoted_and_embedded.csv"
    ),
    source: include_bytes!("../testdata/csv/quoted_and_embedded.csv"),
    extension: "csv",
};

#[tokio::test]
async fn redacts_pii_in_quoted_cells() -> Result<()> {
    let outcome = FIXTURE.run_tabular().await?;

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());

    // The email appears in a plain cell and again inside a quoted cell with an
    // embedded comma / newline / doubled-quote escape — every occurrence gone.
    assert_pii_removed!(
        outcome.redacted_text(),
        "alice.rivera@example.com",
        "bob.nguyen@example.com",
    );

    // The non-sensitive quoted content around the PII survives, including the
    // embedded-comma address and the doubled-quote phrasing.
    assert_content_preserved!(
        outcome.redacted_text(),
        "100 Market St, Suite 400, Springfield",
        "she said",
    );
    Ok(())
}
