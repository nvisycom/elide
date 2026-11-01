//! Text Word stores across several runs (a bold local part, two editing
//! sessions, a proofing mark, a space in a run of its own, a field instruction
//! over two runs) is read whole by detection and redacted, while every run
//! keeps its own properties.

use elide::Result;

use super::BODY_PART;
use crate::support::asserts::{assert_content_preserved, assert_tokens_present};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/docx/split_runs.docx"
    ),
    source: include_bytes!("../testdata/docx/split_runs.docx"),
    extension: "docx",
};

/// Every PII value `docx/split_runs.docx` carries, each split across runs.
const FIXTURE_PII: &[&str] = &[
    "jane.roe@example.com",
    "123-45-6789",
    "+1 (415) 555-0142",
    "+1 (510) 555-0199",
    "GB29 NWBK 6016 1331 9268 19",
    "bob.smith@example.com",
    "192.168.1.42",
];

#[tokio::test]
async fn docx_redacts_text_split_across_runs() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    let body = outcome.part(BODY_PART).expect("body part present");
    let body = String::from_utf8(body).expect("body XML is UTF-8");
    // No fragment of a split value survives either.
    for fragment in [
        "jane.roe@",
        "45-6789",
        "555-0142",
        "1331 9268 19",
        "smith@example.com",
        "1.42<",
    ] {
        assert!(
            !body.contains(fragment),
            "`{fragment}` survived in:\n{body}"
        );
    }
    assert_tokens_present!(
        body,
        "[email_address]",
        "[government_id]",
        "[phone_number]",
        "[iban]",
        "[ip_address]",
    );
    // The runs keep their properties, the tab stays, and the field keeps its
    // instruction and result.
    assert_content_preserved!(
        body,
        "<w:b/>",
        "<w:tab/>",
        "HYPERLINK \"mailto:",
        "Email Bob",
        " for access.",
    );

    outcome.assert_no_pii(FIXTURE_PII);
    Ok(())
}
