//! Catalog label selection: a caller that scopes the request to a subset of
//! entity types gets only those redacted, while every other detectable value
//! is left in place. This pins the `catalog().retain_declared(..)` output
//! filter — the seam that lets a caller opt into exactly the labels they want.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/label_selection.txt"
    ),
    source: include_bytes!("../testdata/txt/label_selection.txt"),
    extension: "txt",
};

#[tokio::test]
async fn a_scoped_catalog_redacts_only_the_requested_label() -> Result<()> {
    // Request only postal codes.
    let outcome = FIXTURE
        .run_with_labels([(*builtins::POSTAL_CODE).clone()])
        .await?;

    // The requested label is redacted.
    assert_pii_removed(&outcome.redacted_text(), &["90210"]);

    // Everything else the pipeline could detect (an email address, a language
    // name) survives, because it is not in the requested catalog.
    assert_preserved(
        &outcome.redacted_text(),
        &["contact@example.com", "Spanish"],
    );
    Ok(())
}

#[tokio::test]
async fn an_empty_catalog_redacts_every_detected_label() -> Result<()> {
    // The default `run` uses an empty catalog, which requests every label.
    let outcome = FIXTURE.run().await?;

    // All three detectable values are redacted — nothing narrows the output.
    assert_pii_removed(
        &outcome.redacted_text(),
        &["90210", "contact@example.com", "Spanish"],
    );
    Ok(())
}
