//! The loader auto-detects the field delimiter (comma, tab, semicolon, pipe).
//! Each fixture carries the same PII under a different delimiter; detection and
//! redaction must work regardless of which one the file uses.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const SEMICOLON: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/csv/delimiter_semicolon.csv"
    ),
    source: include_bytes!("../testdata/csv/delimiter_semicolon.csv"),
    extension: "csv",
};

const PIPE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/csv/delimiter_pipe.csv"
    ),
    source: include_bytes!("../testdata/csv/delimiter_pipe.csv"),
    extension: "csv",
};

const TAB: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/csv/delimiter_tab.csv"
    ),
    source: include_bytes!("../testdata/csv/delimiter_tab.csv"),
    extension: "csv",
};

async fn assert_detected_and_redacted(fixture: Fixture, sep: char) -> Result<()> {
    let outcome = fixture.run_tabular().await?;

    assert_label_present!(outcome.entities, builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present!(outcome.entities, builtins::PHONE_NUMBER.to_ref());

    assert_pii_removed!(
        outcome.redacted_text(),
        "alice.rivera@example.com",
        "bob.nguyen@example.com",
        "+1 (415) 555-0142",
        "+1 (510) 555-0199",
    );

    // The delimiter is preserved on re-serialize: the header row round-trips
    // with the same separator the file used.
    assert_preserved!(
        outcome.redacted_text(),
        format!("name{sep}email{sep}phone").as_str(),
    );
    Ok(())
}

#[tokio::test]
async fn semicolon_delimiter_is_detected() -> Result<()> {
    assert_detected_and_redacted(SEMICOLON, ';').await
}

#[tokio::test]
async fn pipe_delimiter_is_detected() -> Result<()> {
    assert_detected_and_redacted(PIPE, '|').await
}

#[tokio::test]
async fn tab_delimiter_is_detected() -> Result<()> {
    assert_detected_and_redacted(TAB, '\t').await
}
