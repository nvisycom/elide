//! Non-cell text: a workbook's comment, drawing, and relationships parts are
//! surfaced as XML container parts and driven through the markup pipeline, so
//! their PII, element text and hyperlink `Target` values alike, is redacted
//! alongside the cells. Comments are collaboration content: kept and redacted
//! when the hidden-content policy redacts them, removed whole by default.

use elide::Result;
use elide::redaction::{Disposition, HiddenContentPolicy};

use super::{FIXTURE_NON_CELL, NON_CELL_PII};

#[tokio::test]
async fn xlsx_redacts_non_cell_text() -> Result<()> {
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
    let outcome = FIXTURE_NON_CELL.run_tabular_with_policy(policy).await?;

    outcome.assert_no_pii(NON_CELL_PII);
    assert!(
        outcome.part("xl/comments1.xml").is_some(),
        "redacted comments were dropped"
    );
    Ok(())
}

#[tokio::test]
async fn xlsx_removes_comments_by_default() -> Result<()> {
    let outcome = FIXTURE_NON_CELL.run_tabular().await?;

    outcome.assert_no_pii(NON_CELL_PII);
    assert!(
        outcome.part("xl/comments1.xml").is_none(),
        "comments survived"
    );
    Ok(())
}
