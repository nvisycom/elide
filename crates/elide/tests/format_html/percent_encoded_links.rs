//! A link that spells its address or number with percent escapes
//! (`mailto:jane.roe%40example.com`, `tel:%2B1…`) is read as whatever opens
//! it reads it, so the value is detected and redacted over its whole raw
//! spelling, while the escapes around it and a `%` in text stay.

use elide::Result;

use crate::support::asserts::{
    assert_content_preserved, assert_pii_removed, assert_tokens_present,
};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/html/percent_encoded_links.html"
    ),
    source: include_bytes!("../testdata/html/percent_encoded_links.html"),
    extension: "html",
};

#[tokio::test]
async fn html_redacts_percent_encoded_link_values() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let redacted = outcome.redacted_text();

    assert_pii_removed!(redacted, "jane.roe", "example.com", "555%20", "0142");
    assert_tokens_present!(redacted, "[email_address]", "[phone_number]");
    assert_content_preserved!(
        redacted,
        "href=\"mailto:[email_address]?subject=Access%20request\"",
        "href=\"tel:[phone_number]\"",
        "Progress: 50% done, 100%25 checked.",
    );
    Ok(())
}
