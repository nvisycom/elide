//! PII at the very first and last bytes of the file, the last with no trailing
//! newline: the codec must extract and redact edge values like any other.

use elide::Result;

use crate::support::asserts::{assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/boundaries_and_whitespace.txt"
    ),
    source: include_bytes!("../testdata/txt/boundaries_and_whitespace.txt"),
    extension: "txt",
};

#[tokio::test]
async fn edge_values_are_redacted() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The value on byte zero, the mid-file phone, and the final value with no
    // trailing newline are all removed.
    assert_pii_removed!(
        outcome.redacted_text(),
        "first.line@example.com",
        "+1 (415) 555-0142",
        "last.line@example.com",
    );

    // Surrounding prose survives, including the last line's non-PII text.
    assert_preserved!(
        outcome.redacted_text(),
        "opens the file",
        "ends on a value with no trailing newline",
    );
    Ok(())
}
