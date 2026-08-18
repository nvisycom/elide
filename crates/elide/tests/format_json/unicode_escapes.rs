//! `\uXXXX` escape sequences in string values round-trip verbatim — the codec
//! splices raw on-the-wire bytes, so an escape outside a redacted span is never
//! decoded or rewritten — while a plain-ASCII value in the same document is
//! still detected and redacted.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/json/unicode_escapes.json"
    ),
    source: include_bytes!("../testdata/json/unicode_escapes.json"),
    extension: "json",
};

#[tokio::test]
async fn unicode_escapes_survive_and_ascii_pii_redacts() -> Result<()> {
    let outcome = FIXTURE.run().await?;
    let out = outcome.redacted_text();

    assert_label_present(&outcome.entities, &builtins::EMAIL_ADDRESS.to_ref());
    assert_pii_removed(&out, &["alice.johnson@example.com"]);

    // The `\uXXXX` escapes are preserved literally, not decoded to `é`/`—`/`€`.
    assert_preserved(
        &out,
        &["Caf\\u00e9 r\\u00e9sum\\u00e9", "\\u2014", "5\\u20ac"],
    );
    Ok(())
}
