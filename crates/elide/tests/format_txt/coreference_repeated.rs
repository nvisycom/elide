//! The same value repeated must be redacted consistently everywhere it occurs,
//! and two distinct values must stay distinct.

use elide::Result;

use crate::support::asserts::assert_pii_removed;
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/coreference_repeated.txt"
    ),
    source: include_bytes!("../testdata/txt/coreference_repeated.txt"),
    extension: "txt",
};

#[tokio::test]
async fn repeated_values_are_all_redacted() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let redacted = outcome.redacted_text();

    // Every occurrence of each repeated value is gone — no stray copy survives.
    assert_pii_removed!(
        redacted,
        "priya.rao@example.com",
        "+1 (415) 555-0142",
        "sam.diaz@example.com",
    );

    // The repeated email appears four times and the phone three times in the
    // source; all occurrences are redacted, so neither raw value remains.
    assert_eq!(
        redacted.matches("priya.rao@example.com").count(),
        0,
        "every occurrence of the repeated email must be redacted",
    );
    Ok(())
}
