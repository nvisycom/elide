//! An object key is context for its value — the JSON counterpart of a CSV
//! header or an XML element name vouching for its content. Each value here is a
//! weak, context-gated shape with no keyword of its own; only the key lifts it
//! over the threshold. The key is tokenized so `paymentCard`, `postal_code`,
//! `TaxId`, and `bank-account` all read as their words.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/json/key_context.json"
    ),
    source: include_bytes!("../testdata/json/key_context.json"),
    extension: "json",
};

#[tokio::test]
async fn the_key_boosts_its_weak_value() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    for label in [
        builtins::PAYMENT_CARD.to_ref(),
        builtins::POSTAL_CODE.to_ref(),
        builtins::TAX_ID.to_ref(),
        builtins::BANK_ACCOUNT.to_ref(),
    ] {
        assert_label_present(&outcome.entities, &label);
    }

    assert_pii_removed(
        &outcome.redacted_text(),
        &["4111 1111 1111 1111", "90210", "912-85-1234", "000123456789"],
    );
    Ok(())
}
