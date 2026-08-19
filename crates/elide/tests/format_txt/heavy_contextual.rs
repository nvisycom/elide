//! Context-gated values, each with its required keyword inside the window, so
//! the boost lifts every one over the threshold and it is detected + redacted.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/heavy_contextual.txt"
    ),
    source: include_bytes!("../testdata/txt/heavy_contextual.txt"),
    extension: "txt",
};

#[tokio::test]
async fn context_lifts_weak_values_over_threshold() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // Each weak value crosses the threshold thanks to its nearby keyword.
    assert_label_present!(
        outcome.entities,
        builtins::PAYMENT_CARD.to_ref(),
        builtins::BANK_ACCOUNT.to_ref(),
        builtins::GOVERNMENT_ID.to_ref(),
        builtins::PASSPORT_NUMBER.to_ref(),
        builtins::DRIVERS_LICENSE.to_ref(),
        builtins::POSTAL_CODE.to_ref(),
        builtins::API_KEY.to_ref(),
        builtins::IBAN.to_ref(),
    );

    // Every vouched-for value is gone.
    assert_pii_removed!(
        outcome.redacted_text(),
        "4111 1111 1111 1111",
        "000123456789",
        "123-45-6789",
        "C31195855",
        "D1234563",
        "90210",
        "ctx_9f8e7d6c5b4a2b1c0d9e",
        "GB29 NWBK 6016 1331 9268 19",
    );

    // The context keywords themselves are not sensitive and survive.
    assert_preserved!(
        outcome.redacted_text(),
        "payment card",
        "checking account",
        "social security",
        "passport number",
        "driver license",
        "postal code",
    );
    Ok(())
}
