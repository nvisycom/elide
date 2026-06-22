//! End-to-end MP3 codec round-trip: decode → analyze (mock STT enricher) →
//! anonymize → encode.
//!
//! The mock STT backend transcribes nothing, so no entities are detected
//! and the clip round-trips unchanged. The point is to exercise the whole
//! audio path — the codec's `StreamDataReader`/`DataWriter<Audio>`, the
//! `SttEnricher` in the analyze phase, and the `Anonymizer<Audio>` — end to
//! end on real MP3 bytes, the same way the text formats are covered.

mod fixtures;

use fixtures::pipeline::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.mp3"),
    source: include_bytes!("testdata/sample.mp3"),
    extension: "mp3",
};

#[tokio::test]
async fn mp3_round_trips_with_no_detections() {
    let outcome = FIXTURE.run_audio().await;

    // The mock STT backend yields no transcript, so nothing is detected.
    assert!(
        outcome.entities.is_empty(),
        "mock STT detects nothing in audio"
    );
    // The clip decodes and re-encodes to a non-empty MP3.
    assert!(!outcome.redacted.is_empty(), "re-encoded MP3 is non-empty");
}
