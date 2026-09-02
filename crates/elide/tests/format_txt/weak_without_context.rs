//! The same weak shapes as `heavy_contextual`, stripped of any nearby keyword.
//! With nothing to boost them, each must stay below threshold and survive.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_absent};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/weak_without_context.txt"
    ),
    source: include_bytes!("../testdata/txt/weak_without_context.txt"),
    extension: "txt",
};

#[tokio::test]
async fn weak_values_without_context_stay_undetected() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // No keyword vouches for these bare numbers, so the weak bank-account
    // shape must never be flagged.
    assert_label_absent!(outcome.entities, builtins::BANK_ACCOUNT.to_ref());

    // The values survive verbatim — proof they were neither detected nor redacted.
    assert_content_preserved!(
        outcome.redacted_text(),
        "000123456789",
        "000987654321",
        "000555000555",
    );
    Ok(())
}
