//! A locale-scoped pattern fires in a foreign-language document. The prose is
//! not German, but a German postal code (`de-plz`, `languages = ["de"]`) sits
//! beside its German keyword `Postleitzahl`. Language scope is a soft boost
//! signal, not a hard filter, so whatever language the surrounding prose
//! detects as does not suppress the match; the German keyword lifts the weak
//! `0.2` postal score over the threshold.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/cross_language_context.txt"
    ),
    source: include_bytes!("../testdata/txt/cross_language_context.txt"),
    extension: "txt",
};

#[tokio::test]
async fn german_keyword_boosts_a_postal_code_in_english_text() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The German postal code is detected even though the document is English:
    // its German keyword `Postleitzahl` boosts the weak match over threshold.
    assert_label_present(&outcome.entities, &builtins::POSTAL_CODE.to_ref());
    assert_pii_removed(&outcome.redacted_text(), &["10115"]);

    // The surrounding prose carries no other sensitive values.
    assert_preserved(
        &outcome.redacted_text(),
        &["Munich branch", "delivery district"],
    );
    Ok(())
}
