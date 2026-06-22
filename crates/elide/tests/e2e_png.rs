//! End-to-end PNG codec round-trip: decode → analyze → anonymize → encode.
//!
//! Recognition reads OCR text an `OcrEnricher` stamps onto the call; the
//! mock OCR backend recognizes nothing, so nothing is detected and the
//! image round-trips unchanged — exercising the whole image + OCR path on
//! real PNG bytes.

mod fixtures;

use elide::Result;
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.png"),
    source: include_bytes!("testdata/sample.png"),
    extension: "png",
};

#[tokio::test]
async fn png_round_trips_with_no_detections() -> Result<()> {
    let outcome = FIXTURE.run_image().await?;

    // The mock OCR backend yields no text, so nothing is detected.
    assert!(
        outcome.entities.is_empty(),
        "mock OCR detects nothing in the image"
    );
    // The image decodes and re-encodes to a non-empty PNG (the 8-byte PNG
    // signature survives).
    assert!(!outcome.redacted.is_empty(), "re-encoded PNG is non-empty");
    assert_eq!(
        &outcome.redacted[..8],
        b"\x89PNG\r\n\x1a\n",
        "output is still a PNG"
    );
    Ok(())
}
