//! Every strong, self-identifying pattern fires on its own shape, no nearby
//! keyword needed. This pins the high-confidence, no-context detectors.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed};
use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/txt/dense_patterns.txt"
    ),
    source: include_bytes!("../testdata/txt/dense_patterns.txt"),
    extension: "txt",
};

#[tokio::test]
async fn detects_every_strong_pattern() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // Each strong detector fires without any context keyword.
    assert_label_present!(
        outcome.entities,
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::URL.to_ref(),
        builtins::IP_ADDRESS.to_ref(),
        builtins::MAC_ADDRESS.to_ref(),
        builtins::API_KEY.to_ref(),
        builtins::AUTH_TOKEN.to_ref(),
        builtins::IBAN.to_ref(),
        builtins::SWIFT_CODE.to_ref(),
        builtins::CRYPTO_ADDRESS.to_ref(),
        builtins::PRIVATE_KEY.to_ref(),
    );

    // Every sensitive value is gone from the output (some are tokened, some
    // erased, the anonymizer config decides which, but none survives).
    assert_pii_removed!(
        outcome.redacted_text(),
        "dana.well@example.com",
        "ops-team@example.org",
        "https://portal.example.com/account/settings?ref=welcome",
        "192.0.2.44",
        "2001:db8::8a2e:370:7334",
        "3c:22:fb:8a:1e:9d",
        "AKIAIOSFODNN7EXAMPLE",
        "ghp_16C7e42F292c6912E7710c838347Ae178B4a",
        "sk_test_00000000000000000000000000",
        "GB29 NWBK 6016 1331 9268 19",
        "NWBKGB2L",
        "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
        "0x52908400098527886E0F7030069857D2E4169EE7",
        "THISISNOTAREALKEYitisatestfixtureblobAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    );
    Ok(())
}
