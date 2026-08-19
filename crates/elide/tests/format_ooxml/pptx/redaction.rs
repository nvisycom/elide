//! The core round-trip: slide `a:t` text is detected and redacted, and no PII
//! survives in any text-bearing part of the re-packed presentation.

use elide::Result;
use elide::entity::builtins;

use super::{FIXTURE, PII};
use crate::support::asserts::assert_label_present;

#[tokio::test]
async fn pptx_detects_and_redacts_slide_text() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    assert_label_present!(
        outcome.entities,
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::PHONE_NUMBER.to_ref(),
    );

    // The redaction token replaced the slide text.
    let slide = outcome
        .part("ppt/slides/slide1.xml")
        .expect("slide part present");
    assert!(
        String::from_utf8_lossy(&slide).contains("[email_address]"),
        "redaction token missing from slide",
    );

    // No PII value survives in any text-bearing part of the output package.
    outcome.assert_no_pii(PII);

    // The presentation structure survives.
    assert!(
        outcome.part("ppt/presentation.xml").is_some(),
        "presentation part must survive",
    );
    Ok(())
}
