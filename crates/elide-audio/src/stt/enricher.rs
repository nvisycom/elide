//! [`SttEnricher`]: transcribe an audio clip and stamp the transcript onto
//! the call so the text recognizers can read it.
//!
//! The speech-to-text counterpart to language detection: it produces no
//! entities, it *enriches*. On each call it transcribes the [`AudioData`]
//! bytes through its STT backend and stamps the resulting [`Transcription`]
//! onto the call as `Audio`'s [`artifact`]. Recognizers running afterward read
//! the transcript text and resolve each match back to the audio time it was
//! spoken in (see [`Audio`]'s [`TextRecognizable`] impl).
//!
//! [`AudioData`]: crate::modality::AudioData
//! [`artifact`]: elide_core::recognition::Subject::artifact
//! [`Audio`]: crate::modality::Audio
//! [`TextRecognizable`]: elide_core::modality::TextRecognizable

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::enrichment::Enricher;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{RecognizerContext, Subject};
use hipstr::HipStr;

#[cfg(any(test, feature = "mocks"))]
use super::MockBackend;
use super::{SttRequest, SttResponse};
use crate::modality::{Audio, Transcription};

/// An [`Enricher<Audio>`] that transcribes the clip, generic over its STT
/// backend `B` — any [`Backend`] whose request is [`SttRequest`] and whose
/// response is [`SttResponse`].
///
/// Stamps the resulting [`Transcription`] onto the call's artifact. Registered on
/// an `Analyzer<Audio>` ahead of its recognizers, the same way a language
/// detector is registered on a text analyzer. `B` may be a
/// `Metered` wrapper.
#[derive(Clone)]
pub struct SttEnricher<B = ()> {
    /// Optional caller-chosen name, surfaced as this enricher's id so a caller
    /// running more than one transcription enricher can tell them apart. `None`
    /// falls back to the crate name at [`id`](Enricher::id) time.
    name: Option<HipStr<'static>>,
    /// Backend that transcribes the clip.
    backend: B,
}

impl<B> SttEnricher<B>
where
    B: for<'a> Backend<Request<'a> = SttRequest<'a>, Response = SttResponse>,
{
    /// A transcription enricher over `backend`.
    ///
    /// Unnamed by default (its id falls back to the crate name); set a name with
    /// [`with_name`](Self::with_name) when running more than one so their ids
    /// stay distinct.
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            name: None,
            backend,
        }
    }

    /// Set the enricher name, surfaced as its id.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<HipStr<'static>>) -> Self {
        self.name = Some(name.into());
        self
    }
}

#[cfg(any(test, feature = "mocks"))]
impl SttEnricher<MockBackend> {
    /// A transcription enricher over the no-op [`MockBackend`].
    ///
    /// [`MockBackend`]: super::MockBackend
    #[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
    #[must_use]
    pub fn mock() -> Self {
        Self::new(MockBackend::new())
    }
}

impl<B> SttEnricher<B> {
    /// This enricher's name — the one set with
    /// [`with_name`](Self::with_name), or the crate name when unset.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(env!("CARGO_PKG_NAME"))
    }
}

#[async_trait::async_trait]
impl<B> Enricher<Audio> for SttEnricher<B>
where
    B: for<'a> Backend<Request<'a> = SttRequest<'a>, Response = SttResponse>,
{
    fn id(&self) -> ComponentId {
        ComponentId::new(self.name().to_owned(), env!("CARGO_PKG_VERSION"))
    }

    async fn enrich(
        &self,
        subject: &mut Subject<Audio>,
        ctx: &RecognizerContext<'_, Audio>,
    ) -> Result<()> {
        // Already transcribed (a second enricher pass, or a restored artifact on
        // a re-run): leave it, so re-recognition never re-invokes the model.
        if subject.is_enriched() {
            return Ok(());
        }
        let data = subject.data();
        let request = SttRequest {
            audio: &data.bytes,
            format: data.format(),
            language: None,
            correlation_id: ctx.correlation_id(),
        };
        let response = self.backend.call(request).await?;
        subject.set_artifact(Transcription::new(response.segments));
        Ok(())
    }
}

// These tests build a real [`AudioData`] via the `fixtures` module (with `wav`
// for the fixture encoder).
#[cfg(all(test, feature = "fixtures", feature = "wav"))]
mod tests {
    use elide_core::backend::Backend;
    use elide_core::entity::audit::ModelEvent;
    use elide_core::modality::TextRecognizable;
    use elide_core::recognition::Scope;

    use super::*;
    use crate::fixtures;
    use crate::modality::{TranscriptSegment, TranscriptWord};
    use crate::primitive::TimeSpan;
    use crate::stt::SttResponse;

    /// A fixed two-word segment with timings the enricher stamps as a
    /// `Transcription`.
    fn canned_segment() -> TranscriptSegment {
        TranscriptSegment::new(TimeSpan::from_millis(0, 900), "hi Alice").with_words(vec![
            TranscriptWord::new(TimeSpan::from_millis(0, 300), "hi"),
            TranscriptWord::new(TimeSpan::from_millis(300, 900), "Alice"),
        ])
    }

    #[tokio::test]
    async fn enrich_stamps_a_readable_transcript() {
        let enricher = SttEnricher::new(MockBackend::with(vec![canned_segment()])).with_name("stt");
        // A set name flows through to the id; unnamed falls back to the crate.
        assert_eq!(enricher.id().name, "stt");
        assert_eq!(
            SttEnricher::new(MockBackend::new()).id().name,
            env!("CARGO_PKG_NAME")
        );

        let data = fixtures::blank_audio_data();
        let scope = Scope::new();
        let ctx = RecognizerContext::new(&scope);
        let mut subject = Subject::new(data);

        enricher.enrich(&mut subject, &ctx).await.unwrap();

        // Recognizers read the transcript from the call's artifact.
        assert_eq!(
            Audio::as_text(subject.data(), subject.artifact()),
            Some("hi Alice")
        );
        // "Alice" is at bytes 3..8; locate resolves it to the word's time.
        let loc = Audio::locate(3..8, subject.data(), subject.artifact()).expect("range resolves");
        assert_eq!(loc.span.start_millis(), 300);
        assert_eq!(loc.span.end_millis(), 900);
    }

    mockall::mock! {
        /// A spy STT backend whose `call`s are counted and verifiable, for
        /// asserting the enricher's self-skip on a re-run.
        SttSpy {}

        #[async_trait::async_trait]
        impl Backend for SttSpy {
            type Request<'a> = SttRequest<'a>;
            type Response = SttResponse;
            fn provenance(&self) -> ModelEvent;
            #[mockall::concretize]
            async fn call(&self, request: SttRequest<'_>) -> Result<SttResponse>;
        }
    }

    /// The re-run reuse: an enrich over a context already carrying an artifact
    /// (a restored `Transcription` from a prior report) skips the backend
    /// entirely, so re-recognition never re-invokes the STT model.
    #[tokio::test]
    async fn a_present_artifact_skips_the_backend() {
        let mut backend = MockSttSpy::new();
        backend.expect_provenance().returning(|| ModelEvent {
            name: "spy".into(),
            ..ModelEvent::default()
        });
        // The self-skip, asserted as a call cardinality: transcribe fires exactly
        // once across both enrich calls. mockall fails the test on drop if not.
        backend
            .expect_call()
            .times(1)
            .returning(|_| Ok(SttResponse::new(vec![canned_segment()])));

        let enricher = SttEnricher::new(backend);
        let data = fixtures::blank_audio_data();
        let scope = Scope::new();
        let ctx = RecognizerContext::new(&scope);

        // First pass: empty artifact → the backend runs once.
        let mut subject = Subject::new(data.clone());
        enricher.enrich(&mut subject, &ctx).await.unwrap();

        // Re-run: seed the subject with the prior (restored) artifact. The
        // enricher self-skips, transcribe is not called again, enforced by the
        // `.times(1)` above, and the seeded transcript is still readable.
        let restored = subject
            .artifact()
            .cloned()
            .expect("the first pass enriched");
        let mut subject = Subject::new(data).with_artifact(restored);
        enricher.enrich(&mut subject, &ctx).await.unwrap();
        assert_eq!(
            Audio::as_text(subject.data(), subject.artifact()),
            Some("hi Alice")
        );
    }

    /// A restored *empty* `Transcription`, a clip a prior pass transcribed to
    /// silence, still counts as enriched, so the backend is not called again.
    /// Guarding on artifact *presence* rather than emptiness is what makes this
    /// hold.
    #[tokio::test]
    async fn a_restored_empty_artifact_skips_the_backend() {
        let mut backend = MockSttSpy::new();
        backend.expect_provenance().returning(|| ModelEvent {
            name: "spy".into(),
            ..ModelEvent::default()
        });
        // The backend must never run: the seeded (empty) artifact is enrichment.
        backend.expect_call().times(0);

        let enricher = SttEnricher::new(backend);
        let data = fixtures::blank_audio_data();
        let scope = Scope::new();
        let ctx = RecognizerContext::new(&scope);

        // Seed an empty Transcription, the recorded result of a prior pass that
        // found silence. The enricher must treat it as enriched and skip.
        let mut subject = Subject::new(data).with_artifact(Transcription::default());
        enricher.enrich(&mut subject, &ctx).await.unwrap();
        // A present-but-empty artifact reads as `Some("")`, not `None`: the clip
        // *was* enriched (to silence), which is distinct from never-transcribed.
        assert_eq!(Audio::as_text(subject.data(), subject.artifact()), Some(""));
    }
}
