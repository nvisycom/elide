//! PII inside CDATA sections is detected and redacted, not skipped as inert
//! markup. Comments are collaboration content: removed under the default
//! hidden-content policy, and detected and redacted like element text when the policy
//! redacts them. Non-sensitive structure survives either way.

use elide::Result;
use elide::entity::builtins;
use elide::redaction::{Disposition, HiddenContentPolicy};
use elide_core::assert_label_present;

use crate::support::asserts::{assert_content_preserved, assert_pii_removed};
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
async fn comments_are_removed_and_cdata_pii_is_redacted() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present!(outcome.entities, builtins::PAYMENT_CARD.to_ref());

    assert_pii_removed!(
        outcome.redacted_text(),
        "bob.smith@example.com",     // comment
        "+1 (510) 555-0199",         // comment
        "alice.johnson@example.com", // CDATA
        "4111 1111 1111 1111",       // CDATA
        "<!--",
    );
    assert_content_preserved!(
        outcome.redacted_text(),
        "<![CDATA[",
        "]]>",
        "<summary>Onboarding follow-up</summary>",
    );
    Ok(())
}

#[tokio::test]
async fn redacted_comments_keep_their_delimiters() -> Result<()> {
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
    let outcome = FIXTURE.run_with_policy(policy).await?;

    assert_label_present!(outcome.entities, builtins::PHONE_NUMBER.to_ref());
    assert_pii_removed!(
        outcome.redacted_text(),
        "bob.smith@example.com",
        "+1 (510) 555-0199",
    );
    assert_content_preserved!(outcome.redacted_text(), "<!--", "-->");
    Ok(())
}
