//! PII in attribute values is detected and redacted, while element structure
//! and non-sensitive attributes survive. The markup handler surfaces attribute
//! values as their own items, so an `email="…"` / `host="…"` value is redacted
//! in place.

use elide::Result;
use elide::entity::builtins;

use crate::support::asserts::{assert_label_present, assert_pii_removed, assert_preserved};
use crate::support::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xml/attributes.xml"
    ),
    source: include_bytes!("../testdata/xml/attributes.xml"),
    extension: "xml",
};

#[tokio::test]
async fn attribute_values_are_detected_and_redacted() -> Result<()> {
    let outcome = FIXTURE.run().await?;

    // The email, phone, and IP live only in attribute values.
    assert_label_present(&outcome.entities, &builtins::EMAIL_ADDRESS.to_ref());
    assert_label_present(&outcome.entities, &builtins::PHONE_NUMBER.to_ref());
    assert_label_present(&outcome.entities, &builtins::IP_ADDRESS.to_ref());

    assert_pii_removed(
        &outcome.redacted_text(),
        &[
            "alice.johnson@example.com",
            "+1 (415) 555-0142",
            "192.168.1.42",
        ],
    );

    // Structure, attribute names, and non-sensitive text/attributes survive.
    assert_preserved(
        &outcome.redacted_text(),
        &[
            "<person email=",
            "phone=",
            "<server host=",
            "note=\"staging box\"",
            "<label>Primary contact</label>",
            "<plain>no attributes here</plain>",
        ],
    );
    Ok(())
}
