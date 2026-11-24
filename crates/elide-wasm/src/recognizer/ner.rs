//! The NER recognizer building block: a recognizer whose model lives in
//! JavaScript.
//!
//! `elide-ner` ships no model; the backend is caller-provided. On wasm the
//! natural backend is the browser itself — a remote inference endpoint, a
//! transformers.js model, a WebGPU pipeline. [`Recognizer::ner`](super::Recognizer::ner) takes a
//! [`Backend`] whose callback is `(text, labels) => Promise<NerSpan[]>`, so Rust
//! runs the NER recognizer (scoring, alignment, label filtering) around whatever
//! inference the app supplies.

use elide::backend::Backend as BackendTrait;
use elide::entity::audit::ModelId;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::ner::backend::{NerRequest, NerResponse, NerSpan};
use elide::{Error, Result};
use js_sys::Array;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::backend::{Backend, Composed, JsCallback};

/// The component's name, and the model every response of the callback
/// names.
const NAME: &str = "js-callback-ner";

/// One span the JS callback returns, deserialized from a plain object.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsSpan {
    /// The canonical label id (e.g. `person_name`, `organization_name`).
    label: String,
    /// Start byte offset of the span in the request text.
    start: usize,
    /// End byte offset (exclusive).
    end: usize,
    /// Model confidence in `[0, 1]`.
    score: f32,
}

/// A NER backend that defers inference to a JavaScript callback.
pub(crate) struct JsNerBackend(JsCallback);

#[async_trait::async_trait]
impl BackendTrait for JsNerBackend {
    type Request<'a> = NerRequest<'a>;
    type Response = NerResponse;

    async fn call(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        // The callback gets an empty label list for `None` (its own label
        // set) too, so a request for no labels is answered here.
        if request.wants_nothing() {
            return Ok(NerResponse::new(ModelId::new(NAME), Vec::new()));
        }
        let (text, labels) = (request.text, request.labels);
        let spans: Vec<JsSpan> = self
            .0
            .call("NER", move || {
                let labels: Array = labels
                    .into_iter()
                    .flatten()
                    .map(|label| JsValue::from_str(label.id()))
                    .collect();
                Ok(Array::of2(&JsValue::from_str(text), &labels))
            })
            .await?;
        // The callback's byte offsets are untrusted: a reversed range, an end
        // past the text, or an offset mid-character would produce a nonsensical
        // or panicking span downstream. Reject them fail-closed.
        let spans = spans
            .into_iter()
            .map(|s| {
                if s.start > s.end
                    || s.end > text.len()
                    || !text.is_char_boundary(s.start)
                    || !text.is_char_boundary(s.end)
                {
                    return Err(Error::malformed(format!(
                        "NER callback returned an invalid span [{}, {}) for text of {} bytes",
                        s.start,
                        s.end,
                        text.len()
                    )));
                }
                Ok(NerSpan::new(s.label, s.score, s.start..s.end))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(NerResponse::new(ModelId::new(NAME), spans))
    }
}

/// Build a NER recognizer whose inference is `backend`'s JavaScript callback.
pub(super) fn build_ner(backend: Backend) -> NerRecognizer<Composed<JsNerBackend>> {
    NerRecognizer::new(backend.build(&JsNerBackend)).with_name(NAME)
}
