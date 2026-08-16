//! End-to-end XLSX codec round-trip: decode → analyze → anonymize → encode.
//!
//! A workbook is an OPC container whose cell text lives in shared strings
//! (`xl/sharedStrings.xml`) and inline strings (in the sheets). The handler
//! redacts each cell as text while the workbook structure re-packs
//! byte-faithfully. Redacting a shared-string cell de-shares it (the cell
//! becomes an inline string), and a pooled value left with no reference is
//! blanked — so no PII survives anywhere in the output package, not even as an
//! orphaned shared string.

mod fixtures;

use elide::Result;
use elide::entity::builtins;
use fixtures::asserts::assert_label_present;
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.xlsx"),
    source: include_bytes!("testdata/sample.xlsx"),
    extension: "xlsx",
};

/// Every PII value present in the fixture, in shared strings and inline cells.
const PII: &[&str] = &["alice@example.com", "bob@example.com", "123-45-6789"];

#[tokio::test]
async fn xlsx_detects_and_redacts_every_part() -> Result<()> {
    let outcome = FIXTURE.run_tabular().await?;

    // The shipped patterns find the workbook's PII across shared and inline
    // cells: two emails and one government id (the SSN).
    for label in [
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::GOVERNMENT_ID.to_ref(),
    ] {
        assert_label_present(&outcome.entities, &label);
    }

    // The redacted token is present in a sheet (the inline SSN cell), proving the
    // cell text was rewritten in place.
    let sheet1 = outcome
        .part("xl/worksheets/sheet1.xml")
        .expect("sheet1 present");
    assert!(
        String::from_utf8_lossy(&sheet1).contains("[government_id]"),
        "redaction token missing from sheet1",
    );

    // The real guarantee: no PII value survives in ANY part of the output zip —
    // not in a sheet, and not as an orphaned shared string.
    for part in part_names(&outcome.redacted) {
        let bytes = outcome.part(&part).expect("listed part is readable");
        let text = String::from_utf8_lossy(&bytes);
        for pii in PII {
            assert!(!text.contains(pii), "PII `{pii}` survived in part `{part}`",);
        }
    }

    // The output is still a valid workbook: the structural parts pass through.
    assert!(
        outcome.part("xl/workbook.xml").is_some(),
        "workbook part must survive",
    );
    assert!(
        outcome.part("[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    Ok(())
}

const FIXTURE2: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample2.xlsx"),
    source: include_bytes!("testdata/sample2.xlsx"),
    extension: "xlsx",
};

/// The non-cell PII: an email in a cell comment, a phone in a drawing's text.
const NON_CELL_PII: &[&str] = &["carol@example.com", "+1 (510) 555-0199"];

#[tokio::test]
async fn xlsx_redacts_comment_and_drawing_text() -> Result<()> {
    // The workbook's comment and drawing parts are surfaced as XML container
    // parts and driven through the markup pipeline by the orchestrator, so their
    // PII is redacted alongside the cells.
    let outcome = FIXTURE2.run_tabular().await?;

    for part in part_names(&outcome.redacted) {
        let bytes = outcome.part(&part).expect("listed part is readable");
        let text = String::from_utf8_lossy(&bytes);
        for pii in NON_CELL_PII {
            assert!(!text.contains(pii), "PII `{pii}` survived in part `{part}`");
        }
    }
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
