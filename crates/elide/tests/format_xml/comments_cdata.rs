//! PII inside comment bodies and CDATA sections is detected and redacted, not
//! skipped as inert markup, a comment or CDATA is as leak-prone as element
//! text. Non-sensitive structure survives.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xml/comments_cdata.xml"
    ),
    source: include_bytes!("../testdata/xml/comments_cdata.xml"),
    extension: "xml",
};

#[tokio::test]
async fn comment_and_cdata_pii_is_redacted() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present!(outcome.entities, builtins::PHONE_NUMBER.to_ref());
    assert_label_present!(outcome.entities, builtins::PAYMENT_CARD.to_ref());

    assert_pii_removed!(
        outcome.redacted_text(),
        "bob.smith@example.com",     // comment
        "+1 (510) 555-0199",         // comment
        "alice.johnson@example.com", // CDATA
        "4111 1111 1111 1111",       // CDATA
    );

    // The comment and CDATA delimiters and the non-sensitive summary survive.
    assert_content_preserved!(
        outcome.redacted_text(),
        "<!--",
        "-->",
        "<![CDATA[",
        "]]>",
        "<summary>Onboarding follow-up</summary>",
    );
    Ok(())
}
