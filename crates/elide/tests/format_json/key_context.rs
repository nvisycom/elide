//! An object key is context for its value, the JSON counterpart of a CSV
//! header or an XML element name vouching for its content. The postal code (a
//! 5-digit format check), the ITIN, and the bank account are shape-only values
//! that sit below the detection threshold on their own; only the key lifts them
//! over it (the companion `neutral_keys_…` test proves they vanish without it).
//! The payment card is different: its Luhn checksum is evidence in its own
//! right, so it is caught with or without a key. The key is tokenized so
//! `paymentCard`, `postal_code`, `TaxId`, and `bank-account` all read as their
//! words.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{
    assert_content_preserved, assert_label_absent, assert_label_present, assert_pii_removed,
};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/json/key_context.json"
    ),
    source: include_bytes!("../testdata/json/key_context.json"),
    extension: "json",
};

/// The same values under neutral keys (`field1`…) carrying no vouching
/// keyword: the control that proves the boost comes from the *key*, not the
/// value.
const NEUTRAL_FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/json/key_context_neutral.json"
    ),
    source: include_bytes!("../testdata/json/key_context_neutral.json"),
    extension: "json",
};

#[tokio::test]
async fn the_key_boosts_its_weak_value() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present!(
        outcome.entities,
        builtins::PAYMENT_CARD.to_ref(),
        builtins::POSTAL_CODE.to_ref(),
        builtins::TAX_ID.to_ref(),
        builtins::BANK_ACCOUNT.to_ref(),
    );

    assert_pii_removed!(
        outcome.redacted_text(),
        "4111 1111 1111 1111",
        "90210",
        "912-85-1234",
        "000123456789",
    );
    Ok(())
}

#[tokio::test]
async fn neutral_keys_leave_the_shape_only_values_untouched() -> Result<()> {
    // The control for `the_key_boosts_its_weak_value`: under neutral keys
    // (`field1`…) the values carry no context keyword. A value that is only a
    // *shape* — the postal code (a 5-digit format check), the unvalidated ITIN,
    // and the bank account — stays below the detection threshold, undetected and
    // preserved verbatim, so its detection in the keyed test came from the *key*.
    //
    // The payment card is the exception and is checked separately below: it
    // carries a Luhn checksum, which is evidence in its own right, so it is
    // caught with or without a vouching key.
    let outcome = NEUTRAL_FIXTURE.run().await?;

    assert_label_absent!(
        outcome.entities,
        builtins::POSTAL_CODE.to_ref(),
        builtins::TAX_ID.to_ref(),
        builtins::BANK_ACCOUNT.to_ref(),
    );

    assert_content_preserved!(
        outcome.redacted_text(),
        "90210",
        "912-85-1234",
        "000123456789",
    );
    Ok(())
}

#[tokio::test]
async fn a_checksum_valid_card_is_caught_even_under_a_neutral_key() -> Result<()> {
    // Unlike the shape-only values, a Luhn-valid card number is self-sufficient
    // evidence: the checksum lifts it over the threshold, so it is detected and
    // redacted even when its key carries no vouching keyword.
    let outcome = NEUTRAL_FIXTURE.run().await?;

    assert_label_present!(outcome.entities, builtins::PAYMENT_CARD.to_ref());
    assert_pii_removed!(outcome.redacted_text(), "4111 1111 1111 1111");
    Ok(())
}
