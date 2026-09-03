//! An element's *name* is context for its text, the markup counterpart of a
//! JSON key or a CSV header vouching for its value. Each value here is a weak,
//! context-gated shape with no keyword in its own text; only the tag name
//! lifts it over the threshold. The name is tokenized so `paymentCard`,
//! `postal_code`, `TaxId`, and `bank-account` all read as their words.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xml/element_name_context.xml"
    ),
    source: include_bytes!("../testdata/xml/element_name_context.xml"),
    extension: "xml",
};

#[tokio::test]
async fn the_element_name_boosts_its_weak_value() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // camelCase, snake_case, PascalCase, and kebab-case names all tokenize so
    // their keyword (`card`, `postal`, `tax`, `account`) matches on a boundary.
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
