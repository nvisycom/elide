# elide-llm

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

LLM-driven entity recognition over text and images for PII/PHI detection.

## Overview

Some sensitive data resists fixed rules: free-form prose, a face in a photo, a
license plate on a sign. Recognizing it well means asking a model that reads
meaning rather than matching a pattern. This crate provides a recognizer that
drives a large language model (for text) or a vision-language model (for
images) and turns its replies into typed entities, located back in the source
so they can be redacted.

The model is swappable: `RigBackend` (feature `rig`) reaches Ollama, and
OpenAI, Anthropic, and Gemini behind their own features, or you write a
backend for anything else. The prompt is swappable too: the built-in
`DefaultPrompt`, your own `Prompt` impl, or a Jinja2 template file
(`Jinja2Prompt`, feature `jinja2`). A prompt controls wording only; the reply's
shape is fixed per modality, so the model's labels land on the toolkit's
canonical taxonomy whichever model produced them.

```rust
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_llm::LlmRecognizer;
# use elide_core::backend::Backend;
# use elide_core::entity::audit::ModelId;
# use elide_llm::backend::{LlmRequest, LlmResponse};
# use elide_llm::candidates::{Candidates, TextCandidate};
#
# /// Stands in for a real model: finds the name in the text.
# struct MyModel;
#
# #[async_trait::async_trait]
# impl Backend for MyModel {
#     type Request<'a> = LlmRequest<'a, Text>;
#     type Response = LlmResponse<Text>;
#
#     fn provenance(&self) -> ModelId {
#         ModelId { name: "my-model".into(), ..ModelId::default() }
#     }
#
#     async fn call(&self, _request: LlmRequest<'_, Text>) -> elide_core::Result<LlmResponse<Text>> {
#         Ok(LlmResponse::new(Candidates {
#             entities: vec![TextCandidate {
#                 value: "Jane Doe".to_owned(),
#                 description: None,
#                 label: "person_name".to_owned(),
#                 confidence: Some(0.9),
#                 context: Some("Jane Doe lives".to_owned()),
#                 coreference: None,
#             }],
#         }))
#     }
# }

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> elide_core::Result<()> {
let recognizer = LlmRecognizer::with_default(MyModel);

let scope = Scope::new();
let subject = Subject::<Text>::new(TextData::new("Jane Doe lives in Berlin"));
let entities = recognizer.recognize(&subject, &Context::new(&scope)).await?;
assert_eq!(entities[0].label.as_str(), "person_name");
# Ok(())
# }
```

The crate holds:

- **`LlmRecognizer`** — the recognizer, over any LLM backend. It renders the
  prompt, asks the backend for the structured candidate batch, and lifts each
  candidate into an entity whose audit event names the backend's model.
  `recognize_batch` sends one batch for many subjects.
- **`backend`** — the `LlmRequest` / `LlmResponse` contract a backend speaks,
  `RigBackend` (feature `rig`), and `MockBackend` (feature `mocks`).
- **`candidates`** — the reply shape the model fills in: a text candidate
  (the value plus the surrounding context that locates it in the source) or
  an image candidate (a normalized box, ordered and clamped into the image).
- **`prompt`** — the `Prompt` contract, `DefaultPrompt`, and `Jinja2Prompt`.
- **`provider`** (feature `rig`) — which service `RigBackend` calls.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE](../../LICENSE)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
