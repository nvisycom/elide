//! Markup a browser reads differently from an XML parser: attributes after a
//! `/`, unquoted values, and script URLs spelled with HTML references are
//! read as a browser reads them, so their handlers are removed and their PII
//! redacted; a construct a browser ends early refuses the document.

use elide::Result;

use crate::support::asserts::{
    assert_content_preserved, assert_pii_removed, assert_tokens_present,
};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/html/hostile_markup.html"
    ),
    source: include_bytes!("../testdata/html/hostile_markup.html"),
    extension: "html",
};

const EARLY_COMMENT_END: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/html/early_comment_end.html"
    ),
    source: include_bytes!("../testdata/html/early_comment_end.html"),
    extension: "html",
};

#[tokio::test]
async fn html_reads_tags_as_a_browser_does() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let redacted = outcome.redacted_text();

    assert_pii_removed!(
        redacted,
        "onclick",
        "onmouseover",
        "javascript",
        "steal()",
        "jane.roe@example.com",
        "555-0142",
    );
    assert_tokens_present!(redacted, "[email_address]", "[phone_number]");
    assert_content_preserved!(
        redacted,
        "<a>Profile</a>",
        r#"<a title="Account">Card</a>"#,
        "<a>Pay</a>",
        " disabled>",
    );
    Ok(())
}

#[tokio::test]
async fn html_a_browser_reads_differently_is_refused() {
    assert!(EARLY_COMMENT_END.run().await.is_err());
}
