//! MP3 handler + loader: adapt the [`elide_audio`] MP3 engine to the codec's
//! [`Handler`]/[`Loader`] contracts.
//!
//! [`Handler`]: crate::Handler
//! [`Loader`]: crate::Loader

use super::macros::impl_audio_handler;

impl_audio_handler! {
    handler = Mp3Handler,
    loader = Mp3Loader,
    format_id = "elide.audio.mp3",
    audio_format = elide_audio::modality::AudioFormat::Mp3,
    extensions = ["mp3"],
    content_types = ["audio/mpeg"],
}

#[cfg(test)]
mod tests {
    use elide_audio::{AudioBuffer, test_util};

    use super::*;
    use crate::Handler as _;

    #[tokio::test]
    async fn stream_reports_a_duration() {
        let clip = AudioBuffer::open(&test_util::mp3_tone(1)).expect("open");
        let mut h = Mp3Handler::new(clip);
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        assert_eq!(chunk.location.span.start_millis(), 0);
        assert!(
            chunk.location.span.end_millis() > 0,
            "duration should be positive"
        );
        assert!(h.read_next().await.unwrap().is_none());
    }
}
