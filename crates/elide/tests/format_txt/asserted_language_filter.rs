//! The asserted-language locale filter. A German VAT id (`de-vat-id`,
//! `languages = ["de"]`, checksum-validated) is detected on its own shape.
//! When the caller *asserts* a contradicting document language, the
//! German-scoped pattern is suppressed. With no asserted language (the
//! unreliable-detection case), nothing is filtered and the value is detected
//! normally, a detected language never suppresses a match.

use elide::Result;
use elide::entity::builtins;
use elide::primitive::LanguageTag;

use crate::support::asserts::{assert_content_preserved, assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/asserted_language_filter.txt"
    ),
    source: include_bytes!("../testdata/txt/asserted_language_filter.txt"),
    extension: "txt",
};

#[tokio::test]
async fn a_contradicting_asserted_language_suppresses_a_locale_match() -> Result<()> {
    // The caller asserts Spanish; the German-scoped VAT id is filtered out.
    let outcome = FIXTURE
        .run_with_language(LanguageTag::parse("es").unwrap())
        .await?;
    assert_content_preserved!(outcome.redacted_text(), "DE123456788");
    Ok(())
}

#[tokio::test]
async fn no_asserted_language_leaves_the_locale_match_intact() -> Result<()> {
    // With no asserted language (unreliable per-cell detection is never a
    // filter trigger), the German VAT id is detected on its own merit.
    let outcome = FIXTURE.run().await?;
    assert_label_present!(outcome.entities, builtins::TAX_ID.to_ref());
    assert_pii_removed!(outcome.redacted_text(), "DE123456788");
    Ok(())
}
