//! MP3 handler + loader: adapt this crate's MP3 engine to the
//! [`elide_codec`] [`Stream`]/[`Loader`] contracts.
//!
//! [`Stream`]: elide_codec::Stream
//! [`Loader`]: elide_codec::Loader

use super::macros::impl_audio_handler;

impl_audio_handler! {
    handler = Mp3Handler,
    loader = Mp3Loader,
    format_id = "elide.audio.mp3",
    audio_format = crate::modality::AudioFormat::Mp3,
    extensions = ["mp3"],
    content_types = ["audio/mpeg"],
}

#[cfg(test)]
mod tests {
    use elide_codec::Stream as _;

    use super::*;
    use crate::{AudioBuffer, fixtures};

    #[tokio::test]
    async fn stream_reports_a_duration() {
        let clip = AudioBuffer::open(&fixtures::mp3_tone(1), crate::modality::AudioFormat::Mp3)
            .expect("open");
        let h = Mp3Handler::new(clip);
        let chunks = h.chunks().unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].location.span.start_millis(), 0);
        assert!(
            chunks[0].location.span.end_millis() > 0,
            "duration should be positive"
        );
    }
}
