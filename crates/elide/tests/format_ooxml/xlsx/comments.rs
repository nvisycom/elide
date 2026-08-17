//! Non-cell text: a workbook's comment and drawing parts are surfaced as XML
//! container parts and driven through the markup pipeline, so their PII is
//! redacted alongside the cells.

use elide::Result;

use super::{FIXTURE_NON_CELL, NON_CELL_PII};

#[tokio::test]
async fn xlsx_redacts_comment_and_drawing_text() -> Result<()> {
    let outcome = FIXTURE_NON_CELL.run_tabular().await?;

    outcome.assert_no_pii(NON_CELL_PII);
    Ok(())
}
