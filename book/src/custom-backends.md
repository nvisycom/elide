# Custom backends

Elide ships few model backends on purpose: the model you use, and how you
reach it, is yours to choose. A backend of your own is a small type that
implements `Backend` for the request and response of the recognizer or
enricher it serves.

## A NER service

A NER request carries the text, the labels to look for (with descriptions,
for zero-shot models; `None` for a model's built-in set), and the language.
The response holds spans: byte ranges into that text, with a label and a
score.

```rust
use elide::backend::Backend;
use elide::entity::audit::ModelEvent;
use elide::prelude::*;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::ner::backend::{NerRequest, NerResponse, NerSpan};

/// A NER model served over HTTP.
struct NerService {
    client: NerClient,
}

#[elide::async_trait]
impl Backend for NerService {
    type Request<'a> = NerRequest<'a>;
    type Response = NerResponse;

    fn provenance(&self) -> ModelEvent {
        ModelEvent { name: "gliner-multi-v2.1".into(), ..ModelEvent::default() }
    }

    async fn call(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        // `None` asks for the model's built-in label set; `Some` for those labels.
        let predictions = self.client.predict(request.text, request.labels).await?;

        Ok(NerResponse::new(
            predictions
                .into_iter()
                .map(|p| NerSpan::new(p.label, p.score, p.start..p.end))
                .collect(),
        ))
    }
}

let ner = NerRecognizer::new(NerService { client: NerClient::default() });
#
# #[derive(Default)]
# struct NerClient;
# struct Prediction { label: String, score: f32, start: usize, end: usize }
# impl NerClient {
#     async fn predict(&self, _text: &str, _labels: Option<&[elide::entity::Label]>) -> Result<Vec<Prediction>> {
#         Ok(Vec::new())
#     }
# }
```

The spans' labels are Elide's labels, such as `person_name`. A model with
labels of its own maps them in the backend; `LabelMap` helps.

## Batches

The default `call_batch` calls `call` once per request. If the provider
accepts several inputs in one request, override `call_batch` to send them
together: an analysis batches its chunks, so this saves a round trip per
chunk. Return one response per request, in order. If the provider caps the
batch size, wrap the backend in `BatchChunked` rather than splitting batches
yourself; see [Rate limits](rate-limits.md).

## Other models

OCR and speech-to-text backends follow the same pattern:

- An OCR backend takes an `OcrRequest` (the image bytes, format, size, and
  language) and returns an `OcrResponse` of text regions with their boxes.
- A speech-to-text backend takes an `SttRequest` (the audio bytes, format,
  and language) and returns an `SttResponse` of `TranscriptSegment`s with
  their time spans.

Each request's language comes from the call's scope, so a backend whose
model takes a language hint can pass it on.
