//! JSON scalar values — numbers, booleans, and `null` — are parsed and passed
//! through inertly, while string-value PII is still detected and redacted. A
//! non-string scalar carries no PII and must not be corrupted.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/json/scalar_values.json"
    ),
    source: include_bytes!("../testdata/json/scalar_values.json"),
    extension: "json",
};

#[tokio::test]
async fn string_pii_redacts_while_non_string_scalars_survive() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let out = outcome.redacted_text();

    assert_label_present(&outcome.entities, &builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present(&outcome.entities, &builtins::IP_ADDRESS.to_ref());

    assert_pii_removed(&out, &["alice.johnson@example.com", "192.168.1.42"]);

    // Number, float, boolean, and null scalars are untouched and round-trip.
    assert_preserved(
        &out,
        &["\"retries\": 3", "\"ratio\": 0.75", "\"active\": true", "\"deleted\": null"],
    );
    Ok(())
}
