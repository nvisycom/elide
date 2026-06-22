//! End-to-end WAV codec round-trip: decode → analyze → anonymize → encode.
//!
//! Recognition reads a transcript an `SttEnricher` stamps onto the call;
//! the mock STT backend transcribes nothing, so nothing is detected and the
//! clip round-trips unchanged — exercising the whole audio path on real
//! WAV bytes.

mod fixtures;

use elide::Result;
use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.wav"),
    source: include_bytes!("testdata/sample.wav"),
    extension: "wav",
};

#[tokio::test]
async fn wav_round_trips_with_no_detections() -> Result<()> {
    let outcome = FIXTURE.run_audio().await?;

    // The mock STT backend yields no transcript, so nothing is detected.
    assert!(
        outcome.entities.is_empty(),
        "mock STT detects nothing in audio"
    );
    // The clip decodes and re-encodes to a non-empty WAV.
    assert!(!outcome.redacted.is_empty(), "re-encoded WAV is non-empty");
    assert_eq!(&outcome.redacted[..4], b"RIFF", "output is still a WAV");
    Ok(())
}
