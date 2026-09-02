//! Deeply nested elements: PII is found and redacted at any depth, and the
//! whole element tree — indentation, container tags, and the nesting itself —
//! survives.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xml/nested_structure.xml"
    ),
    source: include_bytes!("../testdata/xml/nested_structure.xml"),
    extension: "xml",
};

#[tokio::test]
async fn deeply_nested_values_are_redacted_and_structure_survives() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present!(outcome.entities, builtins::IBAN.to_ref());

    // Both emails (at different depths) and the deeply-nested IBAN are gone.
    assert_pii_removed!(
        outcome.redacted_text(),
        "alice.johnson@example.com",
        "bob.smith@example.com",
        "GB29 NWBK 6016 1331 9268 19",
    );

    // The container structure — every level of nesting — is preserved.
    assert_content_preserved!(
        outcome.redacted_text(),
        "<organization>",
        "<department name=\"Finance\">",
        "<team>",
        "<member>",
        "<records>",
    );
    Ok(())
}
