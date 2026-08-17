//! The core round-trip: every sensitive label is detected across the
//! workbook's shared-string cells, each cell is redacted in place, and the
//! shared-string pool is left with no orphaned PII.

use elide::Result;
use elide::entity::builtins;

use super::FIXTURE;
use crate::format_ooxml::SHARED_PII;
use crate::support::asserts::assert_label_present;

#[tokio::test]
async fn xlsx_detects_and_redacts_every_part() -> Result<()> {
    let outcome = FIXTURE.run_tabular().await?;
    // The shipped patterns find every sensitive label across the workbook's
    // cells — including the payment card, which needs its column header (`card`)
    // as context to reach the detection threshold.
    for label in [
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::PHONE_NUMBER.to_ref(),
        builtins::PAYMENT_CARD.to_ref(),
        builtins::IBAN.to_ref(),
        builtins::GOVERNMENT_ID.to_ref(),
        builtins::IP_ADDRESS.to_ref(),
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

    // The real guarantee: no PII value survives in any text-bearing part of
    // the output package — not in a sheet, and not as an orphaned shared string.
    outcome.assert_no_pii(SHARED_PII);

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
