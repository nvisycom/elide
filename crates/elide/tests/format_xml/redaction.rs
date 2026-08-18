//! End-to-end XML codec round-trip: decode → analyze → anonymize → encode.
//!
//! The markup handler surfaces element text, attribute values, comment
//! bodies, and CDATA payloads; PII in each is redacted while the XML
//! declaration, tags, namespaces, and structure pass through unchanged.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{
    assert_label_present, assert_pii_removed, assert_preserved, assert_tokens_present,
};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/xml/redaction.xml"),
    source: include_bytes!("../testdata/xml/redaction.xml"),
    extension: "xml",
};

#[tokio::test]
async fn xml_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

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

    // PII is gone from every construct the handler surfaces: element text,
    // the `host`/`contact` attribute values, the comment body, and CDATA.
    assert_pii_removed(
        &outcome.redacted_text(),
        &[
            "alice.johnson@example.com",
            "bob.smith@example.com",
            "+1 (415) 555-0142",
            "+1 (510) 555-0199",
            "4111 1111 1111 1111",
            "GB29 NWBK 6016 1331 9268 19",
            "123-45-6789",
            "192.168.1.42",
        ],
    );

    assert_tokens_present(
        &outcome.redacted_text(),
        &[
            "[email_address]",
            "[phone_number]",
            "[iban]",
            "[government_id]",
            "[ip_address]",
        ],
    );

    // Markup structure survives: the declaration, namespaced root, tags, and
    // non-sensitive text stay verbatim.
    assert_preserved(
        &outcome.redacted_text(),
        &[
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<onboarding xmlns=\"urn:example:onboarding\"",
            "<name>Alice Johnson</name>",
            "<status>active</status>",
        ],
    );
    Ok(())
}
