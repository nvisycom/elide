//! Namespaced elements: PII in prefixed elements is detected and redacted, the
//! namespace declarations and prefixes survive, and the element name used for
//! context is the *local* name (so `<c:email>` still reads as `email`).

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xml/namespaces.xml"
    ),
    source: include_bytes!("../testdata/xml/namespaces.xml"),
    extension: "xml",
};

#[tokio::test]
async fn namespaced_elements_redact_and_declarations_survive() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present!(outcome.entities, builtins::PHONE_NUMBER.to_ref());

    assert_pii_removed!(
        outcome.redacted_text(),
        "alice.johnson@example.com",
        "+1 (415) 555-0142",
    );

    // Namespace declarations and prefixed tags round-trip unchanged.
    assert_content_preserved!(
        outcome.redacted_text(),
        "xmlns:ns=\"urn:example:directory\"",
        "xmlns:c=\"urn:example:contact\"",
        "<ns:entry>",
        "<c:email>",
    );
    Ok(())
}
