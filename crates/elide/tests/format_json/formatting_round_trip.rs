//! The JSON codec is a splice, not a re-serializer: irregular whitespace,
//! key order, and indentation are preserved byte-for-byte outside the redacted
//! value spans. Only the detected values change.

use elide::Result;

use crate::support::asserts::{assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/json/formatting_round_trip.json"
    ),
    source: include_bytes!("../testdata/json/formatting_round_trip.json"),
    extension: "json",
};

/// The whole document as it must come out: the two PII values replaced by
/// their tokens, and every other byte — key order, colon/comma spacing, the
/// tab indent, trailing spaces, newlines — identical to the input.
const EXPECTED: &str = include_str!("../testdata/json/formatting_round_trip.expected.json");

#[tokio::test]
async fn irregular_formatting_and_key_order_are_preserved() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let out = outcome.redacted_text();

    assert_pii_removed!(out, "alice.johnson@example.com", "+1 (415) 555-0142");

    // Original key order (zeta before alpha), odd spacing around colons and
    // commas, and the tab indentation all survive verbatim.
    assert_preserved!(
        out,
        "\"zeta\":    \"keep this key order\"",
        "\"nested\":{\"phone\":  ",
        "\t\"alpha\":",
    );

    // The fragment checks above document intent; this pins the *entire* output,
    // so any drift in unredacted whitespace, punctuation, or line endings — not
    // just the sampled fragments — fails the test.
    assert_eq!(out, EXPECTED);
    Ok(())
}
