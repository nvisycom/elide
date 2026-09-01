//! [`SttEnricher`]: transcribe an audio clip and stamp the transcript onto
//! the call so the text recognizers can read it.
//!
//! The speech-to-text counterpart to language detection: it produces no
//! entities, it *enriches*. On each call it transcribes the [`AudioData`]
//! bytes through its [`SttBackend`] and stamps the resulting [`Transcription`]
//! onto the call as `Audio`'s [`artifact`]. Recognizers running afterward read
//! the transcript text and resolve each match back to the audio time it was
//! spoken in (see [`Audio`]'s [`TextRecognizable`] impl).
//!
//! [`AudioData`]: elide_core::modality::audio::AudioData
//! [`artifact`]: elide_core::recognition::RecognizerContext::artifact
//! [`SttBackend`]: crate::SttBackend
//! [`Audio`]: elide_core::modality::audio::Audio
//! [`TextRecognizable`]: elide_core::modality::TextRecognizable

use std::sync::Arc;

use derive_builder::Builder;
use elide_core::modality::audio::{Audio, AudioData, Transcription};
use elide_core::recognition::{Enricher, Enrichment, RecognizerContext, RecognizerId};
use elide_core::{Error, Result};
use hipstr::HipStr;

#[cfg(any(test, feature = "mock"))]
use crate::MockBackend;
use crate::{SttBackend, SttRequest};

/// An [`Enricher<Audio>`] that transcribes the clip.
///
/// Stamps the resulting [`Transcription`] onto the call's artifact. Holds an
/// `Arc<dyn SttBackend>`; cloning shares the backend. Registered on
/// an `Analyzer<Audio>` ahead of its recognizers, the same way a language
/// detector is registered on a text analyzer.
///
/// Built the same way as a NER or LLM recognizer —
/// `SttEnricher::builder().with_name(..).with_backend(..).build()` — so
/// `name` and `backend` are required and construction is uniform across the
/// model-backed components.
#[derive(Clone, Builder)]
#[builder(
    name = "SttEnricherBuilder",
    pattern = "owned",
    setter(into, prefix = "with"),
    build_fn(error = "Error", name = "try_build", private)
)]
pub struct SttEnricher {
    /// Caller-chosen name, surfaced as this enricher's usage id so a caller
    /// running more than one transcription enricher can tell them apart. The
    /// model it calls is reported separately, from the backend's provenance.
    name: HipStr<'static>,
    /// Backend that transcribes the clip. Required. Set via [`with_backend`],
    /// which accepts any concrete [`SttBackend`] impl by value and wraps it in
    /// `Arc` internally.
    ///
    /// [`with_backend`]: SttEnricherBuilder::with_backend
    #[builder(setter(custom))]
    backend: Arc<dyn SttBackend>,
}

impl SttEnricher {
    /// Start the chainable builder. `name` and `backend` are required;
    /// calling [`build`] without them returns a validation error.
    ///
    /// [`build`]: SttEnricherBuilder::build
    #[must_use]
    pub fn builder() -> SttEnricherBuilder {
        SttEnricherBuilder::default()
    }

    /// This enricher's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl SttEnricherBuilder {
    /// Set the [`SttBackend`] that transcribes the clip. Accepts any concrete
    /// impl by value and wraps it in `Arc`. Required: `build` errors when this
    /// hasn't been called.
    #[must_use]
    pub fn with_backend<B: SttBackend>(mut self, backend: B) -> Self {
        self.backend = Some(Arc::new(backend));
        self
    }

    /// Wire the no-op [`MockBackend`] as this enricher's backend.
    ///
    /// [`MockBackend`]: crate::MockBackend
    #[cfg(any(test, feature = "mock"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
    #[must_use]
    pub fn with_mock_backend(self) -> Self {
        self.with_backend(MockBackend)
    }

    /// Finish the builder. Errors when `name` or `backend` is unset.
    pub fn build(self) -> Result<SttEnricher> {
        self.try_build()
    }
}

#[async_trait::async_trait]
impl Enricher<Audio> for SttEnricher {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn enrich(
        &self,
        data: &AudioData,
        ctx: &mut RecognizerContext<'_, Audio>,
    ) -> Result<Enrichment> {
        // Already transcribed (a second enricher pass, or a restored artifact on
        // a re-run): leave it, so re-recognition never re-invokes the model.
        if !ctx.artifact.is_empty() {
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
        ctx.artifact = Transcription::new(response.segments);
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
        let enricher = SttEnricher::builder()
            .with_name("stt")
            .with_backend(CannedBackend)
            .build()
            .expect("builder succeeds");
        // The usage id carries the caller's name, not a fixed crate string.
        assert_eq!(enricher.id().name, "stt");

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

        // Recognizers read the transcript from the call's artifact.
        assert_eq!(Audio::as_text(&data, &ctx.artifact), "hi Alice");
        // "Alice" is at bytes 3..8; locate resolves it to the word's time.
        let loc = Audio::locate(3..8, &data, &ctx.artifact).expect("range resolves");
        assert_eq!(loc.span.start_millis(), 300);
        assert_eq!(loc.span.end_millis(), 900);
    }
}
