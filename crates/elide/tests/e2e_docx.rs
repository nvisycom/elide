//! End-to-end DOCX codec round-trip: decode → analyze → anonymize →
//! encode, over a real `contact.docx` package.
//!
//! Proves the full pipeline redacts the body text in `word/document.xml`
//! while every other zip entry — the embedded image, the relationships,
//! the content-types — survives byte-for-byte. The text path only;
//! embedded-media *redaction* is a separate increment.

mod fixtures;

use fixtures::asserts::{assert_label_present, assert_pii_removed, assert_tokens_present};
use fixtures::pipeline::Fixture;

use elide::entity::builtins;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/contact.docx"),
    source: include_bytes!("testdata/contact.docx"),
    extension: "docx",
};

const BODY_PART: &str = "word/document.xml";
const IMAGE_PART: &str = "word/media/image1.png";

#[tokio::test]
async fn docx_detects_and_redacts_body_text() {
    let outcome = FIXTURE.run_docx().await;

    // The shipped patterns find the same labels they do in the other
    // `contact.*` fixtures.
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

    // Non-body parts survive the round-trip: the embedded image comes
    // back byte-identical to the known PNG the fixture carries, and the
    // other structural parts are still present.
    assert_eq!(
        outcome.part(IMAGE_PART).as_deref(),
        Some(CONTACT_PNG),
        "embedded image must round-trip untouched",
    );
    assert!(
        outcome.part("[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    assert!(
        outcome.part("word/_rels/document.xml.rels").is_some(),
        "relationships part must survive",
    );
}

/// The exact 1×1 PNG `contact.docx` embeds at `word/media/image1.png`.
/// The redaction must return it untouched.
const CONTACT_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];
