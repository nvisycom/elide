//! XML entity references (`&amp;`, `&lt;`, `&gt;`) round-trip verbatim, and a
//! detectable value that sits beside them is still redacted. The handler
//! slices raw on-the-wire bytes, so entities outside a redacted span are
//! spliced back unchanged.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xml/entities_and_escapes.xml"
    ),
    source: include_bytes!("../testdata/xml/entities_and_escapes.xml"),
    extension: "xml",
};

#[tokio::test]
async fn entities_round_trip_and_neighbouring_pii_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let out = outcome.redacted_text();

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());

    // The plain email beside `&lt;`/`&gt;` is removed.
    assert_pii_removed!(out, "carol.lee@example.com");

    // Entity references not inside a redacted span survive verbatim — the
    // standalone `&amp;` in the note text and the `&lt;`/`&gt;` around the plain
    // contact are untouched.
    assert_preserved!(out, "Ampersand &amp; company", "&lt;preferred&gt;");
    Ok(())
}
