//! The NER recognizer building block: a recognizer whose model lives in
//! JavaScript.
//!
//! `elide-ner` ships no model; the backend is caller-provided. On wasm the
//! natural backend is the browser itself — a remote inference endpoint, a
//! transformers.js model, a WebGPU pipeline. [`Recognizer::ner`](super::Recognizer::ner) takes an
//! async JS callback `(text, labels) => Promise<NerSpan[]>` and wraps it in a
//! `JsCallbackBackend`, so Rust runs the NER recognizer (scoring, alignment,
//! label filtering) around whatever inference the app supplies.

use elide::entity::audit::ModelEvent;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::ner::backend::{NerBackend, NerRequest, NerResponse, NerSpan};
use elide::{Error, ErrorKind, Result};
use js_sys::{Array, Function, Promise};
use send_wrapper::SendWrapper;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// One span the JS callback returns, deserialized from a plain object.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsSpan {
    /// The canonical label id (e.g. `person`, `organization`).
    label: String,
    /// Start byte offset of the span in the request text.
    start: usize,
    /// End byte offset (exclusive).
    end: usize,
    /// Model confidence in `[0, 1]`.
    score: f32,
}

/// A [`NerBackend`] that defers inference to a JavaScript callback.
///
/// The callback is a `!Send` [`Function`]; [`SendWrapper`] makes it satisfy the
/// `Send + Sync` bound the recognizer requires. That is sound on wasm, which is
/// single-threaded — the wrapper only panics if accessed from another thread,
/// which never happens here.
struct JsCallbackBackend {
    callback: SendWrapper<Function>,
}

impl JsCallbackBackend {
    fn new(callback: Function) -> Self {
        Self {
            callback: SendWrapper::new(callback),
        }
    }

    /// Invoke the JS callback with `(text, labels)` and await its promise. The
    /// `!Send` work stays inside a [`SendWrapper`] future so it never has to be
    /// `Send` across the await.
    async fn call_js(&self, text: String, labels: Array) -> std::result::Result<JsValue, JsValue> {
        SendWrapper::new(async move {
            let ret = self
                .callback
                .call2(&JsValue::NULL, &JsValue::from_str(&text), &labels)?;
            JsFuture::from(Promise::resolve(&ret)).await
        })
        .await
    }
}

#[async_trait::async_trait]
impl NerBackend for JsCallbackBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "js-callback-ner".into(),
            ..Default::default()
        }
    }

    async fn recognize(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        let labels = Array::new();
        if let Some(requested) = request.labels {
            for label in requested {
                labels.push(&JsValue::from_str(label.id()));
            }
        }
        let value = self
            .call_js(request.text.to_owned(), labels)
            .await
            .map_err(js_to_error)?;
        let spans: Vec<JsSpan> = serde_wasm_bindgen::from_value(value).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("NER callback returned an unreadable value: {e}"),
            )
        })?;
        let spans = spans
            .into_iter()
            .map(|s| NerSpan::new(s.label, s.score, s.start..s.end))
            .collect();
        Ok(NerResponse::new(spans))
    }
}

/// Turn a `JsValue` thrown or rejected by the callback into the crate error.
fn js_to_error(value: JsValue) -> Error {
    let message = value.as_string().unwrap_or_else(|| {
        js_sys::Reflect::get(&value, &JsValue::from_str("message"))
            .ok()
            .and_then(|m| m.as_string())
            .unwrap_or_else(|| "NER callback rejected".to_owned())
    });
    Error::new(ErrorKind::Processing, message)
}

/// Build a NER recognizer whose inference is the JavaScript `callback`.
pub(super) fn build_ner(
    callback: Function,
) -> std::result::Result<NerRecognizer, crate::error::ElideError> {
    Ok(NerRecognizer::builder()
        .with_name("js-callback-ner")
        .with_backend(JsCallbackBackend::new(callback))
        .build()?)
}
