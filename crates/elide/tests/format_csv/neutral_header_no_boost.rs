//! The negative of `column_header_context`: when the header is a neutral word
//! (`serial`, `ref`) rather than a sensitive-field keyword, it provides no
//! boost, so the same weak values stay below threshold and survive.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_absent};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/csv/neutral_header_no_boost.csv"
    ),
    source: include_bytes!("../testdata/csv/neutral_header_no_boost.csv"),
    extension: "csv",
};

#[tokio::test]
async fn neutral_headers_do_not_boost_weak_values() -> Result<()> {
    let outcome = FIXTURE.run_tabular().await?;

    // No sensitive-field keyword names these columns, so the weak bank-account
    // shape is never lifted over the threshold.
    assert_label_absent!(outcome.entities, builtins::BANK_ACCOUNT.to_ref());

    // The bare numbers survive verbatim — neither detected nor redacted.
    assert_content_preserved!(
        outcome.redacted_text(),
        "000123456789",
        "000987654321",
        "000555000555",
        "000456000456",
    );
    Ok(())
}
