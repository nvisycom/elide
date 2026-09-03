//! Adjacent and nested candidates stress reconciliation: every real entity is
//! surfaced once with clean boundaries, and all of them are redacted.

use elide::Result;

use crate::support::asserts::assert_pii_removed;
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/entity_dense_overlap.txt"
    ),
    source: include_bytes!("../testdata/txt/entity_dense_overlap.txt"),
    extension: "txt",
};

#[tokio::test]
async fn overlapping_and_adjacent_entities_all_redacted() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The email nested in a URL, the two run-together values, and the three
    // crowded emails are each removed, nothing leaks through a seam.
    assert_pii_removed!(
        outcome.redacted_text(),
        "leah.kim@example.com",
        "alex.tan@example.com",
        "+1 (628) 555-0175",
        "noor.h@example.com",
        "omar.s@example.com",
        "wei.l@example.com",
    );
    Ok(())
}
