//! An object key is context for its value — the JSON counterpart of a CSV
//! header or an XML element name vouching for its content. The card, postal
//! code, and bank account here are weak, context-gated shapes with no keyword
//! of their own; only the key lifts them over the threshold (the companion
//! `neutral_keys_…` test proves they vanish without it). The SSN-shaped value
//! is self-detecting and rides along to show the key does not *suppress* a
//! strong shape. The key is tokenized so `paymentCard`, `postal_code`,
//! `TaxId`, and `bank-account` all read as their words.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{
    assert_label_absent, assert_label_present, assert_pii_removed, assert_preserved,
};
use crate::support::pipeline::Fixture;

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
        &[
            "4111 1111 1111 1111",
            "90210",
            "912-85-1234",
            "000123456789",
        ],
    );
    Ok(())
}

#[tokio::test]
async fn neutral_keys_leave_the_weak_values_untouched() -> Result<()> {
    // The control for `the_key_boosts_its_weak_value`: the identical values
    // under neutral keys (`field1`…) carry no context keyword, so each stays
    // below the detection threshold — undetected and preserved verbatim. That
    // is what proves the detection in the keyed test comes from the *key*, not
    // the value.
    let outcome = NEUTRAL_FIXTURE.run().await?;

    for label in [
        builtins::PAYMENT_CARD.to_ref(),
        builtins::POSTAL_CODE.to_ref(),
        builtins::TAX_ID.to_ref(),
        builtins::BANK_ACCOUNT.to_ref(),
    ] {
        assert_label_absent(&outcome.entities, &label);
    }

    assert_preserved(
        &outcome.redacted_text(),
        &[
            "4111 1111 1111 1111",
            "90210",
            "912-85-1234",
            "000123456789",
        ],
    );
    Ok(())
}
