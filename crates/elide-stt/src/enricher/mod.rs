//! [`SttEnricher`]: transcribe an audio clip and stamp the transcript onto
//! the call so the text recognizers can read it.
//!
//! The speech-to-text counterpart to language detection: it produces no
//! entities, it *enriches*. On each call it transcribes the [`AudioData`]
//! bytes through its [`SttBackend`] and inserts the resulting
//! [`Transcription`] into the call's
//! [`artifacts`].
//! Recognizers running afterward read the transcript text and resolve each
//! match back to the audio time it was spoken in (see [`Audio`]'s
//! [`TextRecognizable`] impl).
//!
//! [`AudioData`]: elide_core::modality::audio::AudioData
//! [`artifacts`]: elide_core::recognition::RecognizerContext::artifacts
//! [`SttBackend`]: crate::SttBackend
//! [`Audio`]: elide_core::modality::audio::Audio
//! [`TextRecognizable`]: elide_core::modality::TextRecognizable

use std::sync::Arc;

use elide_core::Result;
use elide_core::modality::audio::{Audio, AudioData, Transcription};
use elide_core::recognition::{Enricher, Enrichment, RecognizerContext, RecognizerId};

use crate::{SttBackend, SttRequest};

/// An [`Enricher<Audio>`] that transcribes the clip.
///
/// Stamps the resulting [`Transcription`] onto the call's artifacts. Holds an
/// `Arc<dyn SttBackend>`; cloning shares the backend. Registered on
/// an `Analyzer<Audio>` ahead of its recognizers, the same way a language
/// detector is registered on a text analyzer.
#[derive(Clone)]
pub struct SttEnricher {
    backend: Arc<dyn SttBackend>,
}

impl SttEnricher {
    /// An enricher that transcribes with `backend`.
    pub fn new(backend: impl SttBackend) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }
}

#[async_trait::async_trait]
impl Enricher<Audio> for SttEnricher {
    fn id(&self) -> RecognizerId {
        RecognizerId::new("elide-stt", env!("CARGO_PKG_VERSION"))
    }

    async fn enrich(
        &self,
        data: &AudioData,
        ctx: &mut RecognizerContext<'_, Audio>,
    ) -> Result<Enrichment> {
        // Already transcribed (e.g. a second enricher pass): leave it.
        if ctx.artifacts.contains::<Transcription>() {
            return Ok(Enrichment::none());
        }
        let mut request = SttRequest::new(&data.bytes);
        if let Some(name) = data.filename.as_deref() {
            request = request.with_filename(name);
        }
        if let Some(id) = ctx.correlation_id() {
            request = request.with_correlation_id(id);
        }
        let response = self.backend.transcribe(request).await?;
        ctx.artifacts.insert(Transcription::new(response.segments));
        // The transcription model vouches for its own identity; STT reports no
        // token counts today.
        #[cfg(feature = "usage")]
        return Ok(Enrichment::with_model(self.backend.provenance().into()));
        #[cfg(not(feature = "usage"))]
        Ok(Enrichment::none())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::ModelEvent;
    use elide_core::modality::TextRecognizable;
    use elide_core::modality::audio::{TranscriptSegment, TranscriptWord};
    use elide_core::primitive::TimeSpan;
    use elide_core::recognition::Scope;

    use super::*;
    use crate::SttResponse;

    /// Backend returning a fixed two-word segment with timings.
    #[derive(Clone)]
    struct CannedBackend;

    #[async_trait::async_trait]
    impl SttBackend for CannedBackend {
        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "canned".into(),
                ..ModelEvent::default()
            }
        }

        async fn transcribe(&self, _request: SttRequest<'_>) -> Result<SttResponse> {
            let segment = TranscriptSegment::new(TimeSpan::from_millis(0, 900), "hi Alice")
                .with_words(vec![
                    TranscriptWord::new(TimeSpan::from_millis(0, 300), "hi"),
                    TranscriptWord::new(TimeSpan::from_millis(300, 900), "Alice"),
                ]);
            Ok(SttResponse::new(vec![segment]))
        }
    }

    #[tokio::test]
    async fn enrich_stamps_a_readable_transcript() {
        let enricher = SttEnricher::new(CannedBackend);
        let data = AudioData::new(b"audio".to_vec());
        let scope = Scope::new();
        let mut ctx = RecognizerContext::new(&scope);

        let _enrichment = enricher.enrich(&data, &mut ctx).await.unwrap();

        // The enrichment reports the transcription model that ran, taken from
        // the backend's `provenance()`.
        #[cfg(feature = "usage")]
        {
            let model = _enrichment.model_usage.expect("STT reports its model");
            assert_eq!(model.model, "canned");
        }

        // Recognizers read the transcript from the call's artifacts.
        assert_eq!(Audio::as_text(&data, &ctx.artifacts), "hi Alice");
        // "Alice" is at bytes 3..8; locate resolves it to the word's time.
        let loc = Audio::locate(3..8, &data, &ctx.artifacts).expect("range resolves");
        assert_eq!(loc.span.start_millis(), 300);
        assert_eq!(loc.span.end_millis(), 900);
    }
}
