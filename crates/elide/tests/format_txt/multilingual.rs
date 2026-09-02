//! The payment-card detector's per-language context table: each Luhn-valid card
//! sits beside its language's card keyword, so every one is boosted + redacted.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/multilingual.txt"
    ),
    source: include_bytes!("../testdata/txt/multilingual.txt"),
    extension: "txt",
};

#[tokio::test]
async fn card_context_boosts_across_languages() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present!(outcome.entities, builtins::PAYMENT_CARD.to_ref());

    // The card beside each language's keyword (card / tarjeta / Kreditkarte /
    // carte) is boosted over the threshold and redacted.
    assert_pii_removed!(
        outcome.redacted_text(),
        "4111 1111 1111 1111",
        "5555 5555 5555 4444",
        "4012 8888 8888 1881",
        "6011 1111 1111 1117",
    );
    Ok(())
}
