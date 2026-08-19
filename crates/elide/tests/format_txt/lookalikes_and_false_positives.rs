//! Weak PII-ish shapes that are actually mundane identifiers, with no keyword
//! to vouch for them. A precise pipeline flags none of them.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_absent, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/lookalikes_and_false_positives.txt"
    ),
    source: include_bytes!("../testdata/txt/lookalikes_and_false_positives.txt"),
    extension: "txt",
};

#[tokio::test]
async fn mundane_lookalikes_are_not_flagged() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // No weak-shape identifier here has supporting context, so none is flagged.
    assert_label_absent!(outcome.entities, builtins::BANK_ACCOUNT.to_ref());
    assert_label_absent!(outcome.entities, builtins::GOVERNMENT_ID.to_ref());
    assert_label_absent!(outcome.entities, builtins::PAYMENT_CARD.to_ref());

    assert_preserved!(
        outcome.redacted_text(),
        "000123456789",
        "000987654321",
        "000555",
        "000456000456",
    );
    Ok(())
}
