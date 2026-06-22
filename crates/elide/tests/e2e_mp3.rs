//! End-to-end MP3 codec round-trip: decode → analyze → anonymize → encode.
//!
//! Recognition reads a transcript an `SttEnricher` stamps onto the call;
//! the mock STT backend transcribes nothing, so nothing is detected and the
//! clip round-trips unchanged — exercising the whole audio path on real
//! MP3 bytes.

mod fixtures;

use elide::Result;
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.mp3"),
    source: include_bytes!("testdata/sample.mp3"),
    extension: "mp3",
};

#[tokio::test]
async fn mp3_round_trips_with_no_detections() -> Result<()> {
    let outcome = FIXTURE.run_audio().await?;

    // The mock STT backend yields no transcript, so nothing is detected.
    assert!(
        outcome.entities.is_empty(),
        "mock STT detects nothing in audio"
    );
    // The clip decodes and re-encodes to a non-empty MP3.
    assert!(!outcome.redacted.is_empty(), "re-encoded MP3 is non-empty");
    Ok(())
}
