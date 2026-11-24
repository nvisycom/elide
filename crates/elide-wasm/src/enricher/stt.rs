//! The STT enricher building block: speech-to-text in JavaScript.
//!
//! `elide-audio` ships the STT contract but no engine; the browser supplies one.
//! [`Enricher::stt`](super::Enricher::stt) takes a [`Backend`] whose callback is
//! `(audio) => Promise<Segment[]>` and builds an [`SttEnricher`] over it, so a
//! text recognizer can scan the transcript and matched time spans are silenced.

use elide::backend::Backend as BackendTrait;
use elide::enrichment::stt::{SttEnricher, SttRequest, SttResponse};
use elide::entity::audit::ModelId;
use elide::modality::audio::TranscriptSegment;
use elide::primitive::TimeSpan;
use elide::{Error, Result};
use js_sys::{Array, Uint8Array};
use serde::Deserialize;
use wasm_bindgen::JsValue;

use crate::backend::{Backend, Composed, JsCallback};

/// The component's name, and the model every response of the callback
/// names.
const NAME: &str = "js-callback-stt";

/// One transcript segment the JS callback returns.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Segment {
    /// The transcribed text of the segment.
    text: String,
    /// Segment start, in milliseconds.
    start_ms: u64,
    /// Segment end, in milliseconds.
    end_ms: u64,
}

/// An STT backend that defers transcription to a JavaScript callback.
pub(crate) struct JsSttBackend(JsCallback);

#[async_trait::async_trait]
impl BackendTrait for JsSttBackend {
    type Request<'a> = SttRequest<'a>;
    type Response = SttResponse;

    async fn call(&self, request: SttRequest<'_>) -> Result<SttResponse> {
        let (audio, language) = (request.audio, request.language);
        let segments: Vec<Segment> = self
            .0
            .call("STT", move || {
                let language =
                    language.map_or(JsValue::UNDEFINED, |tag| JsValue::from_str(tag.as_str()));
                Ok(Array::of2(&Uint8Array::from(audio), &language))
            })
            .await?;
        // The callback's time offsets are untrusted: reject a reversed span
        // fail-closed rather than let it be silently clamped to zero length.
        let segments = segments
            .into_iter()
            .map(|s| {
                if s.start_ms > s.end_ms {
                    return Err(Error::malformed(format!(
                        "STT callback returned a reversed span [{} ms, {} ms)",
                        s.start_ms, s.end_ms
                    )));
                }
                Ok(TranscriptSegment::new(
                    TimeSpan::from_millis(s.start_ms, s.end_ms),
                    s.text,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(SttResponse::new(ModelId::new(NAME), segments))
    }
}

/// Build an STT enricher whose transcription is `backend`'s JavaScript callback.
pub(super) fn build_stt(backend: Backend) -> SttEnricher<Composed<JsSttBackend>> {
    SttEnricher::new(backend.build(&JsSttBackend)).with_name(NAME)
}
