//! End-to-end PPTX codec round-trip: decode → analyze → anonymize → encode.
//!
//! A presentation's user text lives as DrawingML `a:t` runs in its slides. The
//! handler redacts that text as it would a DOCX body, and the package re-packs
//! byte-faithfully with only the redacted parts changed.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use fixtures::asserts::assert_label_present;
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.pptx"),
    source: include_bytes!("testdata/sample.pptx"),
    extension: "pptx",
};

/// The PII in the slide's text runs: an email and a phone.
const PII: &[&str] = &["alice@example.com", "+1 (510) 555-0199"];

#[tokio::test]
async fn pptx_detects_and_redacts_slide_text() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    for label in [
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::PHONE_NUMBER.to_ref(),
    ] {
        assert_label_present(&outcome.entities, &label);
    }

    // The redaction token replaced the slide text.
    let slide = outcome
        .part("ppt/slides/slide1.xml")
        .expect("slide part present");
    assert!(
        String::from_utf8_lossy(&slide).contains("[email_address]"),
        "redaction token missing from slide",
    );

    // No PII value survives in any part of the output package.
    for part in part_names(&outcome.redacted) {
        let bytes = outcome.part(&part).expect("listed part is readable");
        let text = String::from_utf8_lossy(&bytes);
        for pii in PII {
            assert!(!text.contains(pii), "PII `{pii}` survived in part `{part}`");
        }
    }

    // The presentation structure survives.
    assert!(
        outcome.part("ppt/presentation.xml").is_some(),
        "presentation part must survive",
    );
    Ok(())
}

/// Every entry name in the redacted zip, so the leak scan covers all parts.
fn part_names(zip_bytes: &[u8]) -> Vec<String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).expect("output is a zip");
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_owned())
        .collect()
}
