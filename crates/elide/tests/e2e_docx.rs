//! End-to-end DOCX codec round-trip: decode → analyze → anonymize → encode.
//!
//! A container format: the body XML is redacted as text and the embedded
//! image is driven by the orchestrator's image pipeline (mock LLM, detects
//! nothing), while the rest of the package (relationships, content-types)
//! passes through unchanged.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use fixtures::asserts::{assert_label_present, assert_pii_removed, assert_tokens_present};
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.docx"),
    source: include_bytes!("testdata/sample.docx"),
    extension: "docx",
};

const BODY_PART: &str = "word/document.xml";
const IMAGE_PART: &str = "word/media/image1.png";

#[tokio::test]
async fn docx_detects_and_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The shipped patterns find the same labels they do in the other
    // `sample.*` fixtures.
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

    // The body XML part: originals gone, replacement tokens in.
    let body = outcome.part(BODY_PART).expect("body part present");
    let body = String::from_utf8(body).expect("body XML is UTF-8");
    assert_pii_removed(
        &body,
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
        &body,
        &[
            "[email_address]",
            "[phone_number]",
            "[iban]",
            "[government_id]",
            "[ip_address]",
        ],
    );

    // The embedded image survives as a valid PNG, and the structural parts
    // are still present.
    let image = outcome.part(IMAGE_PART).expect("image part present");
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n", "image part is not a PNG");
    assert!(
        outcome.part("[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    assert!(
        outcome.part("word/_rels/document.xml.rels").is_some(),
        "relationships part must survive",
    );
    Ok(())
}
