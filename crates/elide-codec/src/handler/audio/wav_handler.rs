//! WAV handler + loader: adapt the [`elide_audio`] WAV engine to the codec's
//! [`Handler`]/[`Loader`] contracts.
//!
//! [`Handler`]: crate::Handler
//! [`Loader`]: crate::Loader

use super::macros::impl_audio_handler;

impl_audio_handler! {
    handler = WavHandler,
    loader = WavLoader,
    format_id = "elide.audio.wav",
    extensions = ["wav"],
    content_types = ["audio/wav", "audio/x-wav"],
}

#[cfg(test)]
mod tests {
    use elide_audio::{AudioBuffer, test_util};

    use super::*;
    use crate::Handler as _;

    #[tokio::test]
    async fn stream_reports_one_second() {
        let clip = AudioBuffer::open(&test_util::wav_ramp(1)).expect("open");
        let mut h = WavHandler::new(clip);
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        assert_eq!(chunk.location.span.start_millis(), 0);
        assert_eq!(chunk.location.span.end_millis(), 1_000);
        assert!(h.read_next().await.unwrap().is_none());
    }
}
