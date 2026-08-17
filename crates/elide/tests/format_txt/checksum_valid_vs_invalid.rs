//! Checksum validators separate a real identifier from a same-shaped number.
//! The valid values are detected; the checksum-failing ones are rejected.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{
    assert_label_absent, assert_label_present, assert_pii_removed, assert_preserved,
};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/checksum_valid_vs_invalid.txt"
    ),
    source: include_bytes!("../testdata/txt/checksum_valid_vs_invalid.txt"),
    extension: "txt",
};

#[tokio::test]
async fn checksum_gates_detection() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The valid card and IBAN are detected...
    assert_label_present(&outcome.entities, &builtins::PAYMENT_CARD.to_ref());
    assert_label_present(&outcome.entities, &builtins::IBAN.to_ref());

    // ...and removed.
    assert_pii_removed(
        &outcome.redacted_text(),
        &[
            "4111 1111 1111 1111",
            "GB29 NWBK 6016 1331 9268 19",
            "5555 5555 5555 4444",
            // The Amex 4-6-5 grouping is a valid card and must be redacted too,
            // not just non-postal (checked below).
            "3782 822463 10005",
        ],
    );

    // The checksum-failing values match the regex but are rejected, so they
    // survive verbatim in the output.
    assert_preserved(
        &outcome.redacted_text(),
        &["4111 1111 1111 1112", "GB29 NWBK 6016 1331 9268 18"],
    );

    // The Amex's 4-6-5 grouping must not spawn a spurious `postal_code` out of
    // its middle digit group — a valid card is one entity, not a card plus a
    // postal code carved from its digits.
    assert_label_absent(&outcome.entities, &builtins::POSTAL_CODE.to_ref());
    Ok(())
}
