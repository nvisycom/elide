//! The STT enricher building block: speech-to-text in JavaScript.
//!
//! `elide-audio` ships the STT contract but no engine; the browser supplies one.
//! [`Enricher::stt`](super::Enricher::stt) takes an async JS callback
//! `(audio) => Promise<Segment[]>` and wraps it in a [`SttBackend`] behind an
//! [`SttEnricher`], so a text recognizer can scan the transcript and matched
//! time spans are silenced.

use elide::enrichment::stt::{SttBackend, SttEnricher, SttRequest, SttResponse};
use elide::entity::audit::ModelEvent;
use elide::modality::audio::TranscriptSegment;
use elide::primitive::TimeSpan;
use elide::{Error, ErrorKind, Result};
use js_sys::{Function, Promise, Uint8Array};
use send_wrapper::SendWrapper;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

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

/// A [`SttBackend`] that defers transcription to a JavaScript callback.
///
/// The callback is a `!Send` [`Function`]; [`SendWrapper`] makes it satisfy the
/// `Send + Sync` bound the enricher requires — sound on single-threaded wasm.
struct JsCallbackBackend {
    callback: SendWrapper<Function>,
}

impl JsCallbackBackend {
    fn new(callback: Function) -> Self {
        Self {
            callback: SendWrapper::new(callback),
        }
    }

    /// Invoke the callback with the audio bytes and await its promise, keeping
    /// the `!Send` work inside a [`SendWrapper`] future.
    async fn call_js(&self, audio: Vec<u8>) -> std::result::Result<JsValue, JsValue> {
        SendWrapper::new(async move {
            let bytes = Uint8Array::from(audio.as_slice());
            let ret = self.callback.call1(&JsValue::NULL, &bytes)?;
            JsFuture::from(Promise::resolve(&ret)).await
        })
        .await
    }
}

#[async_trait::async_trait]
impl SttBackend for JsCallbackBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "js-callback-stt".into(),
            ..Default::default()
        }
    }

    async fn transcribe(&self, request: SttRequest<'_>) -> Result<SttResponse> {
        let value = self
            .call_js(request.audio.to_vec())
            .await
            .map_err(super::js_to_error)?;
        let segments: Vec<Segment> = serde_wasm_bindgen::from_value(value).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("STT callback returned an unreadable value: {e}"),
            )
        })?;
        // The callback's time offsets are untrusted: reject a reversed span
        // fail-closed rather than let it be silently clamped to zero length.
        let segments = segments
            .into_iter()
            .map(|s| {
                if s.start_ms > s.end_ms {
                    return Err(Error::new(
                        ErrorKind::MalformedInput,
                        format!(
                            "STT callback returned a reversed span [{} ms, {} ms)",
                            s.start_ms, s.end_ms
                        ),
                    ));
                }
                Ok(TranscriptSegment::new(
                    TimeSpan::from_millis(s.start_ms, s.end_ms),
                    s.text,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(SttResponse::new(segments))
    }
}

/// Build an STT enricher whose transcription is the JavaScript `callback`.
pub(super) fn build_stt(
    callback: Function,
) -> std::result::Result<SttEnricher, crate::error::ElideError> {
    Ok(SttEnricher::builder()
        .with_name("js-callback-stt")
        .with_backend(JsCallbackBackend::new(callback))
        .build()?)
}
