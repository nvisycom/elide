//! The column header is CSV's context signal: a header like `account` or `card`
//! boosts the weak value in its column over the threshold, the way a nearby
//! keyword does in prose. This is the mechanic CSV adds on top of raw detection.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/csv/column_header_context.csv"
    ),
    source: include_bytes!("../testdata/csv/column_header_context.csv"),
    extension: "csv",
};

#[tokio::test]
async fn header_boosts_weak_column_values() -> Result<()> {
    let outcome = FIXTURE.run_tabular().await?;

    // The `account` header lifts the weak bank-account shape; the `card` header
    // lifts the (Luhn-valid) card. Both are detected because of their column.
    assert_label_present(&outcome.entities, &builtins::BANK_ACCOUNT.to_ref());
    assert_label_present(&outcome.entities, &builtins::PAYMENT_CARD.to_ref());

    // Every value in the account and card columns is removed across all rows.
    assert_pii_removed(
        &outcome.redacted_text(),
        &[
            "000123456789",
            "000987654321",
            "000555000555",
            "4111 1111 1111 1111",
            "5555 5555 5555 4444",
            "4012 8888 8888 1881",
        ],
    );

    // The header row and non-sensitive name cells survive.
    assert_preserved(
        &outcome.redacted_text(),
        &["name,account,card,note", "Alice Rivera,", "primary"],
    );
    Ok(())
}
