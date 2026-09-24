//! WAV handler + loader: adapt this crate's WAV engine to the
//! [`elide_codec`] [`Handler`]/[`Loader`] contracts.
//!
//! [`Handler`]: elide_codec::Handler
//! [`Loader`]: elide_codec::Loader

use super::macros::impl_audio_handler;

impl_audio_handler! {
    handler = WavHandler,
    loader = WavLoader,
    format_id = "elide.audio.wav",
    audio_format = crate::modality::AudioFormat::Wav,
    extensions = ["wav"],
    content_types = ["audio/wav", "audio/x-wav"],
}

#[cfg(test)]
mod tests {
    use elide_codec::content::ContentData;
    use elide_codec::{Handler as _, Loader as _};

    use super::*;
    use crate::{AudioBuffer, test_util};

    #[tokio::test]
    async fn stream_reports_one_second() {
        let clip = AudioBuffer::open(&test_util::wav_ramp(1)).expect("open");
        let mut h = WavHandler::new(clip);
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        assert_eq!(chunk.location.span.start_millis(), 0);
        assert_eq!(chunk.location.span.end_millis(), 1_000);
        assert!(h.read_next().await.unwrap().is_none());
    }

    /// The WAV loader rejects content that opens as another format: MP3 bytes
    /// must not decode into a `WavHandler` just because `AudioBuffer::open`
    /// accepts them.
    #[cfg(feature = "mp3")]
    #[tokio::test]
    async fn loader_rejects_content_of_another_format() {
        let err = WavLoader
            .decode(ContentData::new(test_util::mp3_tone(1)))
            .await
            .expect_err("mp3 content should not decode as wav");
        assert_eq!(err.kind(), elide_core::ErrorKind::MalformedInput);
    }
}
