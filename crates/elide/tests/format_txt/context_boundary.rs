//! The context window has an edge: a keyword lifts a weak value only within a
//! few words. The in-window bank account is detected; the out-of-window one is
//! not.
//!
//! The value is a bare bank-account number: a shape with no checksum that stays
//! below the detection threshold on its own, so only a nearby keyword surfaces
//! it. (A value carrying its own checksum, a payment card, would self-fire
//! regardless of the window and so cannot probe this boundary.)

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

    // The account close to "account" is boosted over the threshold and redacted.
    assert_pii_removed!(outcome.redacted_text(), "123456789012");

    // The account too far from "account" stays weak and survives verbatim.
    assert_content_preserved!(outcome.redacted_text(), "987654321098");
    Ok(())
}
