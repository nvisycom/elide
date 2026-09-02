//! The context window has an edge: a keyword lifts a weak value only within a
//! few words. The in-window card is detected; the out-of-window one is not.

use elide::Result;

use crate::support::asserts::{assert_content_preserved, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/context_boundary.txt"
    ),
    source: include_bytes!("../testdata/txt/context_boundary.txt"),
    extension: "txt",
};

#[tokio::test]
async fn keyword_boosts_only_within_the_window() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The card close to "card" is boosted over the threshold and redacted.
    assert_pii_removed!(outcome.redacted_text(), "4111 1111 1111 1111");

    // The card too far from "card" stays weak and survives verbatim.
    assert_content_preserved!(outcome.redacted_text(), "5555 5555 5555 4444");
    Ok(())
}
