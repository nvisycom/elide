//! `\uXXXX` escape sequences in string values round-trip verbatim, the codec
//! splices raw on-the-wire bytes, so an escape outside a redacted span is never
//! decoded or rewritten, while a plain-ASCII value in the same document is
//! still detected and redacted.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_content_preserved, assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

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

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());
    assert_pii_removed!(out, "alice.johnson@example.com");

    // The `\uXXXX` escapes are preserved literally, not decoded to `é`/`,`/`€`.
    assert_content_preserved!(out, "Caf\\u00e9 r\\u00e9sum\\u00e9", "\\u2014", "5\\u20ac",);
    Ok(())
}

#[tokio::test]
async fn pii_after_escapes_redacts_while_the_escaped_prefix_survives() -> Result<()> {
    // The `greeting` value decodes to `😀 café corner: bob.smith@example.com`:
    // a leading surrogate pair (`😀`) and a BMP escape (`é`)
    // precede the email. Detection sees the *decoded* value, so the email is
    // found; redaction must then map the value-space span back through those
    // escapes to the right *source* bytes, replacing only the email and
    // leaving every escape in the prefix byte-for-byte intact.
    let outcome = FIXTURE.run().await?;
    let out = outcome.redacted_text();

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());
    // The email (past the escapes) is gone…
    assert_pii_removed!(out, "bob.smith@example.com");
    // …while the surrogate pair and BMP escape before it survive verbatim,
    // proving the source-offset mapping counted escape bytes correctly rather
    // than shifting into or past the prefix.
    assert_content_preserved!(out, "\\uD83D\\uDE00 caf\\u00e9 corner: ");
    Ok(())
}
