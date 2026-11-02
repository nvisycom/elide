//! Text an HTML paragraph splits across inline elements, or spells with
//! character references, is read whole by detection and redacted, while the
//! inline elements, their attributes, a line break, and the untouched
//! references pass through.

use elide::Result;

use crate::support::asserts::{
    assert_content_preserved, assert_pii_removed, assert_tokens_present,
};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/html/split_inline.html"
    ),
    source: include_bytes!("../testdata/html/split_inline.html"),
    extension: "html",
};

#[tokio::test]
async fn html_redacts_text_split_across_inline_elements() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let redacted = outcome.redacted_text();

    assert_pii_removed!(
        redacted,
        "jane.roe@",
        "45-6789",
        "555-0142",
        "192.168.",
        "jane&#46;roe",
        "example.org",
    );
    assert_tokens_present!(
        redacted,
        "[email_address]",
        "[government_id]",
        "[phone_number]",
        "[ip_address]",
    );
    assert_content_preserved!(
        redacted,
        "<b>",
        "<span class=\"id\">",
        "<a href=\"tel:office\">",
        "<br>Rack 7</p>",
        "Backup:&nbsp;[email_address]",
        " for access.",
    );
    Ok(())
}
