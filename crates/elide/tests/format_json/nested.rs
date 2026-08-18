//! Nested objects and arrays: PII is found and redacted at any depth and
//! inside array elements (object values and bare string array items alike),
//! while the container structure survives.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/json/nested.json"),
    source: include_bytes!("../testdata/json/nested.json"),
    extension: "json",
};

#[tokio::test]
async fn nested_objects_and_arrays_redact_at_any_depth() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present(&outcome.entities, &builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present(&outcome.entities, &builtins::IBAN.to_ref());

    // Emails in object values (nested), a bare string in an array, and the
    // deeply-nested IBAN are all removed.
    assert_pii_removed(
        &outcome.redacted_text(),
        &[
            "alice.johnson@example.com",
            "bob.smith@example.com",
            "carol.lee@example.com",
            "GB29 NWBK 6016 1331 9268 19",
        ],
    );

    // Container keys and a non-PII array item survive.
    assert_preserved(
        &outcome.redacted_text(),
        &["\"organization\"", "\"departments\"", "\"members\"", "not-an-email"],
    );
    Ok(())
}
