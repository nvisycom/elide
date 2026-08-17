//! End-to-end suite for the audio formats (WAV, MP3). They share one audio
//! handler (decode → silence/remove span → re-encode) and the STT
//! enrichment path, so they share one test binary. Each format is a
//! scenario module under `format_audio/`.

#[path = "support/mod.rs"]
mod support;

#[path = "format_audio/mod.rs"]
mod format_audio;
