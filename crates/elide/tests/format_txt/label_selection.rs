//! Catalog label selection: the request catalog names the entity types to
//! detect. A scoped subset redacts only those types; an empty catalog requests
//! nothing, so nothing is detected or redacted. This pins the catalog as the
//! caller's opt-in for exactly the labels they want.

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
    assert_pii_removed!(outcome.redacted_text(), "90210");

    // Everything else the pipeline could detect (an email address, a language
    // name) survives, because it is not in the requested catalog.
    assert_preserved!(outcome.redacted_text(), "contact@example.com", "Spanish",);
    Ok(())
}

#[tokio::test]
async fn an_empty_catalog_detects_nothing() -> Result<()> {
    // An empty catalog requests no entity types, so the analyzer detects
    // nothing — every detectable value is left in place.
    let outcome = FIXTURE.run_with_labels([]).await?;

    assert_preserved!(
        outcome.redacted_text(),
        "90210",
        "contact@example.com",
        "Spanish",
    );
    Ok(())
}
