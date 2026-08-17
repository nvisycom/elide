//! The core round-trip: every sensitive label is detected, and every part
//! that carries text — body, header, and relationship targets — is redacted.

use elide::Result;
use elide::entity::builtins;

use super::{BODY_PART, FIXTURE, HEADER_PART, RELS_PART};
use crate::format_ooxml::SHARED_PII;
use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_tokens_present};

#[tokio::test]
async fn docx_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The shipped patterns find every sensitive label across the document.
    for label in [
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::PHONE_NUMBER.to_ref(),
        builtins::PAYMENT_CARD.to_ref(),
        builtins::IBAN.to_ref(),
        builtins::GOVERNMENT_ID.to_ref(),
        builtins::IP_ADDRESS.to_ref(),
    ] {
        assert_label_present(&outcome.entities, &label);
    }

    // The body part: originals gone, replacement tokens in.
    let body = outcome.part(BODY_PART).expect("body part present");
    let body = String::from_utf8(body).expect("body XML is UTF-8");
    assert_tokens_present(
        &body,
        &[
            "[email_address]",
            "[phone_number]",
            "[iban]",
            "[government_id]",
            "[ip_address]",
        ],
    );

    // The header carries an email too, and it is redacted in its own part.
    let header = outcome.part(HEADER_PART).expect("header part present");
    let header = String::from_utf8(header).expect("header XML is UTF-8");
    assert_pii_removed(&header, &["alice.johnson@example.com"]);
    assert_tokens_present(&header, &["[email_address]"]);

    // The body email is also an external hyperlink `mailto:` target in the
    // relationships part; redaction reaches it there too.
    let rels = outcome
        .part(RELS_PART)
        .expect("relationships part must survive");
    let rels = String::from_utf8(rels).expect("relationships XML is UTF-8");
    assert_pii_removed(&rels, &["bob.smith@example.com"]);
    assert_tokens_present(&rels, &["[email_address]"]);

    // The real guarantee: no PII value survives in any text-bearing part of
    // the output package.
    outcome.assert_no_pii(SHARED_PII);

    // The content-types manifest still round-trips.
    assert!(
        outcome.part("[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    Ok(())
}
